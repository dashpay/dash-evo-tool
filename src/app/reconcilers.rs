//! Per-frame reconcilers extracted from [`AppState`](super::AppState).
//!
//! Each reconciler owns the fields it needs and exposes an `update()`/action
//! method the frame loop calls once per frame, mirroring the `ThemeState`
//! precedent. Reconcilers that must dispatch async work return the
//! [`BackendTask`] for `AppState` to run through its channel, keeping the
//! dispatch chokepoint in one place.
//!
//! Policy, user-facing copy, and state transitions live with their reconciler.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dash_sdk::dpp::dashcore::{Network, Txid};
use eframe::egui;

use crate::backend_task::error::TaskError;
use crate::backend_task::migration::{MigrationTask, migration_task_error};
use crate::backend_task::{BackendTask, platform_info};
use crate::context::AppContext;
use crate::context::connection_status::{
    OverallConnectionState, SPV_SYNC_PHASE_COUNT, spv_phase_step, spv_progress_token,
};
use crate::context::migration_status::{MigrationState, MigrationStep};
use crate::model::wallet::{TransactionConfirmation, TransactionStatus, WalletSeedHash};
use crate::ui::MessageType;
use crate::ui::components::wallet_unlock_popup::{
    MigrationWalletUnlockResult, WalletUnlockPopup, wallet_needs_unlock,
};
use crate::ui::components::{
    BannerHandle, MessageBanner, OptionBannerExt, OptionOverlayExt, OverlayConfig, OverlayHandle,
};
use crate::ui::identity::identity_pill::shorten_id;

use super::{AppAction, BootPhase};

/// Banner action id pushed when the user clicks "Retry now" on the
/// migration-failure banner. The app loop matches this id and
/// re-dispatches the FinishUnwire backend task. Kept as a `const` so a
/// future second migration variant can pick a distinct id without
/// risking a typo collision. Exposed for kittest coverage.
pub const MIGRATION_RETRY_ACTION_ID: &str = "migration:retry:finish_unwire";

/// Banner action id pushed when the user acknowledges the warning about
/// unreadable balance top-up records. Until it fires, the warning is re-raised
/// on every launch — a dismissed banner is not an acknowledgement. Exposed for
/// kittest coverage.
pub const MIGRATION_APP_DATA_ACK_ACTION_ID: &str = "migration:ack:unreadable_app_data";

/// Banner action id pushed when the user acknowledges the unreadable-identity
/// warning. Until it fires, the warning is re-raised on every launch — a
/// dismissed banner is not an acknowledgement, because the identities it names
/// hold keys the user cannot sign with until they are loaded again. Exposed for
/// kittest coverage.
pub const MIGRATION_IDENTITIES_ACK_ACTION_ID: &str = "migration:ack:unreadable_identities";

/// Banner action id pushed when the user acknowledges the combined warning — the
/// launch where both unreadable identities and unreadable top-up records were
/// left behind.
/// One banner names both problems, so its single acknowledgement retires both
/// records: re-raising either half after the user has read and dismissed the
/// sentence describing it would be a notice they have already acted on. Exposed
/// for kittest coverage.
pub const MIGRATION_UNREADABLE_ACK_ACTION_ID: &str =
    "migration:ack:unreadable_identities_and_app_data";

/// Action id for the SPV-sync block's "Cancel" button.
///
/// SPV sync is **unbounded** — with no peers it stays Connecting/Syncing forever
/// with no terminal signal — so a button-less hard block would trap the user
/// (violating the overlay's C1/C2 contract). Cancel is also designated the
/// block's single keyboard-reachable escape (`with_keyboard_escape`), so a
/// keyboard-only / assistive-tech user can activate it with Enter or Space.
///
/// Because that makes a *destructive* action reachable by the universal dismiss
/// key, for users who may not be able to see what they are dismissing, Cancel
/// does not stop sync: it swaps the action row for the confirmation below. One
/// reflexive keypress is therefore never destructive.
/// Colon-namespaced per the overlay action-id convention. Exposed for kittest
/// coverage.
pub const SPV_CANCEL_ACTION_ID: &str = "spv:sync:cancel";

/// Action id for the "Stop syncing" button of the Cancel confirmation.
/// Activating it disconnects; it is deliberately NOT the keyboard escape.
pub const SPV_CANCEL_CONFIRM_ACTION_ID: &str = "spv:sync:cancel_confirm";

/// Action id for the "Keep syncing" button of the Cancel confirmation — the
/// non-destructive default, and the confirmation's keyboard escape, so Enter or
/// Space returns to the block rather than disconnecting.
pub const SPV_CANCEL_KEEP_ACTION_ID: &str = "spv:sync:cancel_keep";

/// The Cancel confirmation's question. Names the consequence and the way back,
/// so the choice is answerable without seeing the screen behind it.
pub const SPV_CANCEL_QUESTION: &str = "Stop syncing? You can start again from the Network screen.";

/// Plain, jargon-free descriptions for the SPV-sync block (Everyday-User rule:
/// no "SPV"/"headers"/"masternodes"/raw heights/percentages — the jargon-free
/// "Step N of 5" counter carries the granularity). Complete sentences (NFR-2).
const SPV_CONNECTING_DESCRIPTION: &str = "Connecting to the Dash network.";
const SPV_SYNCING_DESCRIPTION: &str = "Syncing with the Dash network.";

/// What the per-frame SPV-sync block driver should do with the overlay this
/// frame, given whether a startup/Connect sync is **armed** and the current
/// connection state. Pure so the policy is unit-testable in isolation from
/// `AppState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpvBlockStep {
    /// Armed and still connecting/syncing: raise (or keep + update).
    Block,
    /// Armed episode reached a terminal state (Synced/Error): lower the block and
    /// DISARM, so subsequent ambient Connecting/Syncing (reconnect, per-block
    /// catch-up) never re-blocks (F-SPV-A).
    Disarm,
    /// Not armed (ambient sync, or already disarmed): ensure no block is shown.
    Idle,
}

/// Pure SPV-sync block policy (F-SPV-A scope gate + C1/C2). The block is **scoped
/// to user-initiated sync** — armed only on startup auto-start and the Connect
/// button — so an ambient reconnect or the SPV engine flipping Synced→Syncing on
/// each new block never hard-blocks a working user. Once an armed episode reaches
/// a terminal state it disarms and stays disarmed until the next Connect/startup.
fn spv_block_step(armed: bool, state: OverallConnectionState) -> SpvBlockStep {
    use OverallConnectionState as S;
    if !armed {
        return SpvBlockStep::Idle;
    }
    match state {
        // Terminal for an armed episode: lower and disarm (banner surfaces Error).
        S::Synced | S::Error => SpvBlockStep::Disarm,
        // Still getting connected/synced for this episode. Disconnected stays
        // blocking while armed — it just means we are still trying to connect.
        S::Connecting | S::Syncing | S::Disconnected => SpvBlockStep::Block,
    }
}

/// One-sentence user-facing label for an in-progress migration step.
/// Mirrors Diziet §2.2 D-1 banner copy — single complete sentence per
/// variant so i18n extraction is trivial. Exposed for kittest
/// coverage so a regression in the label table fails the test suite.
pub fn migration_running_text(step: MigrationStep) -> &'static str {
    match step {
        MigrationStep::Wiring => "The app is opening your saved data.",
        MigrationStep::Detecting => "The app is checking your wallet data.",
        MigrationStep::AppData => "The app is restoring your identity balance history.",
        MigrationStep::SingleKey => "The app is updating your imported keys.",
        MigrationStep::Shielded => "The app is verifying your shielded balance.",
        MigrationStep::WalletSeeds => "The app is moving your wallets into secure storage.",
        MigrationStep::WalletMeta => "The app is updating your wallet names.",
        MigrationStep::Identities => "The app is restoring your identities and their keys.",
        MigrationStep::Finalize => "The app is finishing the storage update.",
    }
}

/// User-facing banner copy for a migration that finished the wallet drain but
/// could not decode `count` identities. Their keys are therefore not loaded, so
/// the sentence names both the screen and the control that restore them — a user
/// who has never opened that flow cannot act on "load them again" alone. The
/// previous version's data is never deleted, so the re-import is always
/// possible. No "Retry now": a corrupt row decodes no better on a second pass.
/// Exposed for kittest coverage.
pub fn migration_unreadable_identities_text(count: u32) -> String {
    format!(
        "Some identities from the previous version could not be read and were not carried over \
         ({count} in total). Your previous data is untouched. For a user identity, choose Load \
         Identity on the Identities screen. For a masternode or evonode identity, choose + Load \
         on the Masternodes tab."
    )
}

/// User-facing banner copy for the rare launch where both DET-owned passes broke:
/// `count` identities could not be read AND updating the rest of the previous
/// version's data (such as balance top-up records) hit a hard error. Names each problem
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

/// User-facing banner copy for every combination of unreadable legacy identity
/// and top-up rows. One sentence per problem, each naming its own remedy.
/// Exposed for kittest coverage.
pub fn migration_unreadable_data_text(identities: u32, top_ups: u32) -> String {
    match (identities > 0, top_ups > 0) {
        (true, false) => migration_unreadable_identities_text(identities),
        (false, true) => format!(
            "Some records of earlier additions to identity balances from the previous version \
             could not be read and were not carried over ({top_ups} in total). Check each \
             identity's balance history before adding more funds."
        ),
        (true, true) => format!(
            "Some identities ({identities} in total) and some records of earlier additions to \
             identity balances ({top_ups} in total) from the previous version could not be read \
             and were not carried over. For a user identity, choose Load Identity on the \
             Identities screen. For a masternode or evonode identity, choose + Load on the \
             Masternodes tab. Check each identity's balance history before adding more funds."
        ),
        (false, false) => {
            "Some data from the previous version could not be read. Check your identities and identity balance history before continuing.".to_string()
        }
    }
}

/// How long storage preparation may run before the gate offers a way out.
///
/// Preparation is local, non-network work (open the SQLite sidecar, run the
/// schema ladder, hydrate wallets, drain the previous version's rows) that
/// normally completes within a few frames of boot — sub-second in the common
/// case. 30 seconds is ~two orders of magnitude past that, generous enough never
/// to false-positive on a slow disk or a large wallet set, yet short enough that
/// a genuinely wedged persister (or a second process holding the file lock)
/// surfaces within half a minute instead of never. It sits well below the
/// network-bound waits (the 120 s SPV no-progress watchdog, the 10 min MCP sync
/// gate), matching that this wait is local, not on the wire. The bound matters
/// more than it used to: the gate has no background escape, so a hang here would
/// otherwise be unrecoverable.
const STORAGE_PREP_STUCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Overlay copy once preparation exceeds [`STORAGE_PREP_STUCK_TIMEOUT`].
/// Everyday-User copy (no "backend"/"wiring"/"SPV" jargon): what happened plus
/// the two self-serviceable choices. Complete sentences so i18n extracts each as
/// one unit.
const STORAGE_PREP_STUCK_MESSAGE: &str = "Preparing your saved data is taking longer than usual. \
     Wait a little longer, or close the app and open it again.";

/// Overlay copy for a preparation that failed and can be retried. Names what
/// happened, reassures about the data, and offers both actions the surface has.
const STORAGE_PREP_FAILED_MESSAGE: &str = "We couldn't finish updating your saved data. Your \
     wallets and keys are unchanged. Try again, or close the app and open it later.";

/// Overlay copy while preparation waits for a migrated wallet's password. The
/// password prompt itself renders above the gate and carries the controls; this
/// is what the card says once the prompt closes. Exposed for kittest coverage:
/// the gate's contract is that this is NOT painted while the prompt is up, and a
/// test asserting that has to name the exact string the card would show.
pub const STORAGE_PREP_PASSWORD_DESCRIPTION: &str =
    "Enter your wallet password to continue the storage update.";

/// Action id for the storage-preparation gate's "Try again" button.
pub const STORAGE_PREP_RETRY_ACTION_ID: &str = "storage:prepare:retry";

/// Action id for the storage-preparation gate's "Close the app" button. Also the
/// gate's single keyboard-reachable exit, so a keyboard-only / assistive-tech
/// user is never stranded behind a block with no background escape.
pub const STORAGE_PREP_CLOSE_ACTION_ID: &str = "storage:prepare:close";

/// How often a pending-confirmation watch re-reads the display snapshot. The
/// snapshot only changes when a wallet event lands, so a faster tick would buy
/// nothing but a repeated scan of every wallet's history.
const PENDING_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long a watch stays unconfirmed before its banner is re-worded once.
/// dash-spv rebroadcasts a pending transaction every 600 s and Dash targets a
/// block every 2.5 min, so by eleven minutes the network has had a full retry
/// cycle and several block windows — past which "wait a moment" is no longer
/// true. The watch itself continues; only the copy changes.
const PENDING_STALE_AFTER: Duration = Duration::from_secs(11 * 60);

/// Cap on simultaneously watched transactions, so a pathological run of
/// ambiguous sends cannot grow the watch list without bound. The oldest watch
/// is retired to [`PENDING_STALE_MESSAGE`], which points at the durable
/// transaction-history row rather than going silent.
const MAX_PENDING_WATCHES: usize = 8;

/// Replaces the ambiguous-outcome banner once the network has demonstrably
/// taken the transaction. Everyday-User copy: the whole outcome in one plain
/// sentence, with nothing left for the user to do.
///
/// Names the transaction because up to [`MAX_PENDING_WATCHES`] payments can be
/// waiting at once and the warning for the others stays up beside this. An
/// unattributed "your transaction is confirmed" next to an unattributed "could
/// not be verified" leaves the user to guess which is which, and guessing wrong
/// in either direction ends in paying the same person twice. The id is
/// shortened the way every other identifier in the app is, and its head and
/// tail still match by eye against the full id in the transaction history the
/// stale copy points at.
fn pending_confirmed_message(txid: &Txid) -> String {
    let transaction = shorten_id(&txid.to_string());
    format!("Your transaction {transaction} is confirmed.")
}

/// Replaces the ambiguous-outcome banner once a watch passes
/// [`PENDING_STALE_AFTER`]. Deliberately promises nothing about the funds
/// becoming spendable again — upstream releases them on a block-height
/// schedule this app does not measure — and hands the user the transaction
/// history, which tracks the same data live, instead of an instruction to act
/// blind. It names no status either: this copy is reached for every
/// non-confirmed observation, including one no loaded wallet has seen at all,
/// so the history row it points at may not exist and may not read the way a
/// named status would promise. Complete sentences so i18n extracts it as one
/// unit.
const PENDING_STALE_MESSAGE: &str = "The app still cannot verify whether your transaction was confirmed. Check the transaction history for the wallet you sent from before sending it again, because sending now could pay the same person twice if the first one arrives later.";

/// What the pending-confirmation watch should do about one watched transaction
/// this tick. Pure so the policy is unit-testable in isolation from `AppState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingStep {
    /// No verdict yet: keep the ambiguous-outcome banner and keep watching.
    Waiting,
    /// The network took it: replace the banner and retire the watch.
    Confirmed,
    /// Unconfirmed for long enough that the original advice has expired:
    /// re-word the banner once and keep watching.
    Stale,
}

/// Pure pending-confirmation policy. `confirmation` is the watched
/// transaction's state in the display snapshot (`None` when no loaded wallet
/// has seen it), `elapsed` how long the watch has been open, and
/// `already_stale` whether the re-wording has already happened — so the copy
/// changes at most once.
///
/// There is deliberately no failure outcome. Modern Dash Core has no rejection
/// signal on the wire, so an invalid transaction and a slow one are
/// indistinguishable; a synthesised "failed" would eventually tell someone
/// their money is safe to send again when it is not.
///
/// TODO(R-1, docs/gui-testing/scenarios/pending-broadcast-auto-reconcile.md):
/// still unverified against a live network — no funded testnet wallet is
/// available in this environment. `network_took_transaction`'s
/// `height.is_some()` guard is the code-side mitigation for a locally-injected
/// transaction reaching `Confirmed` without a real network verdict, and that
/// guard is what the scenario exists to exercise. Unit tests cover the policy
/// against a synthetic snapshot; what is outstanding is confirming that a real
/// dash-spv mempool injection is in fact reported height-less, which only a
/// funded run can show. Also closes R-4 (no-change-output send shape).
fn pending_step(
    confirmation: Option<TransactionConfirmation>,
    elapsed: Duration,
    already_stale: bool,
) -> PendingStep {
    if confirmation.is_some_and(network_took_transaction) {
        PendingStep::Confirmed
    } else if !already_stale && elapsed >= PENDING_STALE_AFTER {
        PendingStep::Stale
    } else {
        PendingStep::Waiting
    }
}

/// Whether a snapshot entry is evidence the network actually took the
/// transaction, rather than evidence this app tried to send it.
///
/// Presence alone proves nothing: dash-spv injects a broadcast transaction into
/// its own mempool before any peer verdict, so an `Unconfirmed` entry appears
/// even for one no peer ever accepted. An InstantSend lock is driven only by a
/// real lock message, and a genuinely mined transaction carries the height of
/// the block holding it — a mined tier without one is not evidence worth
/// spending a funds message on.
fn network_took_transaction(confirmation: TransactionConfirmation) -> bool {
    match confirmation.status {
        TransactionStatus::Unconfirmed => false,
        TransactionStatus::InstantSendLocked => true,
        TransactionStatus::Confirmed | TransactionStatus::ChainLocked => {
            confirmation.height.is_some()
        }
    }
}

/// Drives platform-level accessibility (AccessKit) activation on the first
/// frames so tooling can see the tree without a live assistive client.
pub(super) struct AccessibilityActivator {
    /// Force-enable requested via `DASH_EVO_TOOL_ACCESSIBILITY=1`.
    enforced: bool,
    /// Whether activation has already succeeded.
    activated: bool,
    /// Frames spent attempting activation.
    retries: u32,
}

impl AccessibilityActivator {
    /// Give up after this many frames to avoid indefinite repaints.
    const MAX_RETRIES: u32 = 60;

    pub(super) fn new(enforced: bool) -> Self {
        Self {
            enforced,
            activated: false,
            retries: 0,
        }
    }

    /// Attempt activation this frame (no-op once enforced-off, activated, or
    /// out of retries). Requests another frame while still retrying.
    pub(super) fn update(&mut self, ctx: &egui::Context) {
        if !self.enforced || self.activated || self.retries >= Self::MAX_RETRIES {
            return;
        }
        self.retries += 1;
        self.activated = crate::platform::force_accessibility_activation();
        if self.activated {
            return;
        }
        if self.retries >= Self::MAX_RETRIES {
            tracing::warn!(
                "Accessibility activation failed after {} frames, giving up",
                Self::MAX_RETRIES
            );
        } else {
            // Ensure another frame to retry, even if egui would go idle.
            ctx.request_repaint();
        }
    }
}

/// Drives the blocking SPV-sync overlay (F-SPV-A). Owns the overlay handle, the
/// armed-episode flag so an ambient reconnect never hard-blocks a working user,
/// and whether the block is currently asking the user to confirm cancelling.
pub(super) struct SpvBlockReconciler {
    /// The blocking overlay raised while an armed sync is connecting.
    overlay: Option<OverlayHandle>,
    /// Whether a user-initiated sync episode is armed for blocking.
    armed: bool,
    resync: Option<crate::backend_task::BackendTaskContext>,
    /// Whether the block is showing the Cancel confirmation rather than progress.
    confirming_cancel: bool,
}

impl SpvBlockReconciler {
    pub(super) fn new(armed: bool) -> Self {
        Self {
            overlay: None,
            armed,
            resync: None,
            confirming_cancel: false,
        }
    }

    /// Arm a fresh user-initiated episode (boot auto-start, Connect button,
    /// post-onboarding auto-start).
    pub(super) fn arm(&mut self) {
        self.armed = true;
        self.resync = None;
        self.confirming_cancel = false;
    }

    /// Keep the block until this exact wallet rescan task finishes.
    pub(super) fn arm_resync(&mut self, context: crate::backend_task::BackendTaskContext) {
        self.arm();
        self.resync = Some(context);
    }

    pub(super) fn finish_resync(&mut self, context: &crate::backend_task::BackendTaskContext) {
        if self.resync.as_ref() == Some(context) {
            self.reset();
        }
    }

    /// Whether an episode is currently armed (test seam / observation).
    #[cfg(feature = "testing")]
    pub(super) fn armed(&self) -> bool {
        self.armed
    }

    /// Whether the block overlay is currently raised — the connection banner
    /// suppresses its redundant Connecting/Syncing copy while it is.
    pub(super) fn is_overlaying(&self) -> bool {
        self.overlay.is_some()
    }

    /// Drop the overlay and disarm the episode (used on network switch).
    pub(super) fn reset(&mut self) {
        self.overlay.take_and_clear();
        self.resync = None;
        self.armed = false;
        self.confirming_cancel = false;
    }

    /// Drive the blocking SPV-sync overlay for one frame (see the field docs on
    /// [`AppState`](super::AppState) for the F-SPV-A / C1-C2 contract). Raises
    /// at most once per episode, then updates content in place.
    ///
    /// Returns [`AppAction::StopSpv`] on the frame the user confirms cancelling.
    /// The action travels back to the frame loop rather than being applied here
    /// because stopping sync is dispatch work `AppState` owns; the reconciler
    /// only decides that it was asked for.
    pub(super) fn update(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
    ) -> Option<AppAction> {
        let cs = app_context.connection_status();
        let state = cs.overall_state();
        let decision = if self.resync.is_some() {
            SpvBlockStep::Block
        } else {
            spv_block_step(self.armed, state)
        };
        match decision {
            SpvBlockStep::Block => {
                // F-SPV-B: plain, jargon-free copy — the determinate granularity
                // is the "Step N of 5" counter, NOT raw phase names / heights.
                let progress = cs.spv_sync_progress();
                let step = progress.as_ref().and_then(spv_phase_step);
                // A-1 (Item B): the hidden liveness token tracks the advancing
                // height so a slow-but-advancing phase never trips the
                // no-progress watchdog. It is never rendered.
                let token = progress.as_ref().and_then(spv_progress_token);
                let description = if self.confirming_cancel {
                    SPV_CANCEL_QUESTION
                } else if self.resync.is_some() {
                    "Rescanning your wallet’s Dash Core history. Keep the app open until the scan finishes."
                } else if step.is_some() {
                    SPV_SYNCING_DESCRIPTION
                } else {
                    SPV_CONNECTING_DESCRIPTION
                };
                if self.overlay.is_none() {
                    let mut config = OverlayConfig::new().with_description(description);
                    config = Self::apply_actions(config, self.confirming_cancel);
                    if let Some(n) = step {
                        config = config.with_step(n, SPV_SYNC_PHASE_COUNT);
                    }
                    if let Some(t) = token {
                        config = config.with_progress_token(t);
                    }
                    self.overlay.raise(ctx, "", config);
                } else if let Some(handle) = &self.overlay {
                    handle.set_description(description);
                    match step {
                        Some(n) => {
                            handle.set_step(n, SPV_SYNC_PHASE_COUNT);
                        }
                        None => {
                            handle.clear_step();
                        }
                    }
                    if let Some(t) = token {
                        handle.set_progress_token(t);
                    }
                }
            }
            SpvBlockStep::Disarm => {
                // Armed episode ended (Synced/Error): lower and disarm so
                // ambient Connecting/Syncing never re-blocks (F-SPV-A).
                self.overlay.take_and_clear();
                self.armed = false;
                self.confirming_cancel = false;
            }
            SpvBlockStep::Idle => {
                // Not armed (ambient sync, or already disarmed): never block.
                self.overlay.take_and_clear();
                self.confirming_cancel = false;
            }
        }

        self.drain_actions()
    }

    /// Put the action row for the current step on `config`.
    ///
    /// The keyboard escape is bound to the *non-destructive* choice in both
    /// rows: Cancel (which only asks) while syncing, "Keep syncing" while
    /// confirming. That is what keeps the block keyboard-reachable without
    /// making Enter or Space disconnect the wallet.
    fn apply_actions(config: OverlayConfig, confirming_cancel: bool) -> OverlayConfig {
        if confirming_cancel {
            config
                .with_action("Stop syncing", SPV_CANCEL_CONFIRM_ACTION_ID)
                .with_secondary_action("Keep syncing", SPV_CANCEL_KEEP_ACTION_ID)
                .with_keyboard_escape(SPV_CANCEL_KEEP_ACTION_ID)
        } else {
            config
                .with_secondary_action("Cancel", SPV_CANCEL_ACTION_ID)
                .with_keyboard_escape(SPV_CANCEL_ACTION_ID)
        }
    }

    /// Drain this overlay's own clicks and apply the two-step cancel.
    ///
    /// Switching between the progress row and the confirmation row lowers the
    /// overlay so the next frame re-raises it: an [`OverlayHandle`]'s button
    /// methods append, and re-asserting a row per frame would stack duplicates.
    fn drain_actions(&mut self) -> Option<AppAction> {
        let actions = self
            .overlay
            .as_ref()
            .map(|handle| handle.take_actions())
            .unwrap_or_default();

        for action in actions {
            if action == SPV_CANCEL_ACTION_ID {
                self.confirming_cancel = true;
                self.overlay.take_and_clear();
            } else if action == SPV_CANCEL_KEEP_ACTION_ID {
                self.confirming_cancel = false;
                self.overlay.take_and_clear();
            } else if action == SPV_CANCEL_CONFIRM_ACTION_ID {
                tracing::info!("User cancelled chain sync from the startup block");
                self.confirming_cancel = false;
                self.overlay.take_and_clear();
                self.armed = false;
                self.resync = None;
                return Some(AppAction::StopSpv);
            }
        }
        None
    }
}

#[derive(Default)]
struct TransientBanner(Option<BannerHandle>);

impl TransientBanner {
    fn track(&mut self, handle: BannerHandle) {
        self.0 = Some(handle);
    }

    fn reset(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.clear();
        }
    }

    fn clear_if(&mut self, condition: bool) {
        if condition {
            self.reset();
        }
    }
}

/// Reconciles the connection-status banner with the overall connection state.
pub(super) struct ConnectionBanner {
    /// Previous state, to detect transitions. `None` forces re-evaluation.
    previous_state: Option<OverallConnectionState>,
    /// Handle to the current connection banner, if displayed.
    handle: Option<BannerHandle>,
    /// Startup proof error cleared once quorum keys become available.
    quorum_startup_error: TransientBanner,
}

impl ConnectionBanner {
    pub(super) fn new() -> Self {
        Self {
            previous_state: None,
            handle: None,
            quorum_startup_error: TransientBanner::default(),
        }
    }

    /// Adopt a quorum-not-ready error banner from the generic task fallback.
    pub(super) fn track_quorum_startup_error(&mut self, handle: BannerHandle) {
        self.quorum_startup_error.track(handle);
    }

    /// Clear the banner and force re-evaluation next frame (network switch).
    pub(super) fn reset(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.clear();
        }
        self.quorum_startup_error.reset();
        self.previous_state = None;
    }

    /// Update the banner for the current connection state. `spv_overlaying`
    /// suppresses the redundant Connecting/Syncing copy while the SPV block is
    /// up; `onboarding_active` suppresses the initial `Disconnected` banner while
    /// the Welcome screen is showing (pre-sync, not a real failure). Returns a
    /// [`BackendTask`] to dispatch on the first `Synced`.
    pub(super) fn update(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
        spv_overlaying: bool,
        onboarding_active: bool,
    ) -> Option<BackendTask> {
        let connection_status = app_context.connection_status();
        self.quorum_startup_error
            .clear_if(connection_status.masternodes_ready());
        let current_state = connection_status.overall_state();
        let state_changed = self.previous_state != Some(current_state);

        // In Connecting state the banner text can change (normal → degraded)
        // without a transition, so re-evaluate every frame; skip otherwise.
        if !state_changed && current_state != OverallConnectionState::Connecting {
            return None;
        }

        // While the SPV-sync block is up it already conveys Connecting/Syncing
        // with live progress, so suppress the redundant banner text.
        if spv_overlaying
            && matches!(
                current_state,
                OverallConnectionState::Connecting | OverallConnectionState::Syncing
            )
        {
            if let Some(handle) = self.handle.take() {
                handle.clear();
            }
            self.previous_state = Some(current_state);
            return None;
        }

        // The Welcome screen initially reads Disconnected before sync starts.
        if onboarding_active && current_state == OverallConnectionState::Disconnected {
            if let Some(handle) = self.handle.take() {
                handle.clear();
            }
            // Invalidate rather than cache either state so suppression exit and
            // recurring pre-suppression states both force reconciliation.
            self.previous_state = None;
            return None;
        }

        // Clear old banner on state transitions.
        if state_changed && let Some(handle) = self.handle.take() {
            handle.clear();
        }

        let mut task = None;
        match current_state {
            OverallConnectionState::Disconnected => {
                let msg = "Disconnected — check your internet connection";
                let handle = MessageBanner::set_global(ctx, msg, MessageType::Error);
                handle.disable_auto_dismiss();
                self.handle = Some(handle);
            }
            OverallConnectionState::Connecting => {
                // SPV active but no peers yet. The degraded flag flips after
                // 30 s — `set_global` is idempotent for same text.
                let msg = if connection_status.spv_peer_degraded() {
                    "Having trouble finding peers. Check your connection."
                } else {
                    "Looking for peers…"
                };
                if let Some(handle) = &self.handle {
                    handle.set_message(msg);
                } else {
                    self.handle = Some(MessageBanner::set_global(ctx, msg, MessageType::Warning));
                }
            }
            OverallConnectionState::Syncing => {
                let msg = "SPV sync in progress…";
                self.handle = Some(MessageBanner::set_global(ctx, msg, MessageType::Warning));
            }
            OverallConnectionState::Error => {
                let handle = MessageBanner::set_global(
                    ctx,
                    "SPV sync failed. Go to Settings for connection details.",
                    MessageType::Error,
                );
                handle.disable_auto_dismiss();
                if let Some(detail) = connection_status.spv_last_error() {
                    handle.with_details(detail);
                }
                self.handle = Some(handle);
            }
            OverallConnectionState::Synced => {
                // No banner. Fetch epoch info on first sync to populate protocol
                // version and fee multiplier (needed for feature gating).
                if state_changed {
                    task = Some(BackendTask::PlatformInfo(
                        platform_info::PlatformInfoTaskRequestType::CurrentEpochInfo,
                    ));
                }
            }
        }
        self.previous_state = Some(current_state);
        task
    }
}

/// Drives the blocking storage-preparation gate: the one place boot decides a
/// network's storage is usable.
///
/// Backend wiring, the legacy drain and chain sync used to race; the gate makes
/// them a sequence by refusing to hand the app back to the user — or to chain
/// sync — until [`AppContext::prepare_storage`] has returned. A thread-blocking
/// gate is impossible (the main thread already sits inside `runtime.block_on`),
/// so the block is on the *user*: preparation runs as a spawned task and this
/// reconciler polls it, owning the whole interaction surface meanwhile.
///
/// The overlay is raised through [`ProgressOverlay`], not a bespoke surface, so
/// it inherits the four-point contract that lets the storage update's own
/// password prompt render, focus and type *above* the block — without which the
/// gate would deadlock on the frame loop its own completion depends on.
pub(super) struct StoragePrepGate {
    /// What may render this frame. The frame loop reads it; only this reconciler
    /// advances it.
    phase: BootPhase,
    /// Networks whose storage this process has already prepared. Returning to
    /// one never re-raises the gate — sentinels are per-network, but the work
    /// behind them is done for this run.
    prepared: BTreeSet<Network>,
    /// The in-flight preparation, if any.
    pending: Option<PendingPrepare>,
    /// Terminal failure surface, once preparation has failed.
    failure: Option<PrepareFailure>,
    /// The blocking overlay raised for the current phase.
    overlay: Option<OverlayHandle>,
    /// Whether to start chain sync once the current preparation completes.
    auto_start_spv: bool,
    /// Whether this is startup preparation, whose sync owns the blocking SPV
    /// overlay. Network-switch sync remains ambient.
    arm_spv_block: bool,
    /// Whether the "raised over nothing" surface has already been armed, so its
    /// log and its overlay swap happen once rather than every frame.
    orphaned: bool,
    #[cfg(feature = "testing")]
    test_hold: Option<tokio::sync::oneshot::Sender<Result<(), TaskError>>>,
}

/// One spawned [`AppContext::prepare_storage`] run.
struct PendingPrepare {
    network: Network,
    started: Instant,
    result: tokio::sync::oneshot::Receiver<Result<(), TaskError>>,
    /// Whether the stuck-preparation copy has already been surfaced.
    stuck: bool,
}

/// A preparation that failed, held so the terminal surface survives re-renders.
struct PrepareFailure {
    network: Network,
    /// Whether retrying can plausibly help. A version-window mismatch cannot be
    /// retried into success, so its surface offers only "Close the app".
    retryable: bool,
    error: TaskError,
}

/// What the frame loop must do after driving the gate for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GateEvent {
    /// Preparation completed: build the root screens and release the app.
    Prepared {
        network: Network,
        start_spv: bool,
        arm_spv_block: bool,
    },
    /// The user chose "Try again" on a failed preparation.
    Retry(Network),
    /// The user chose "Close the app" on a terminal failure.
    Close,
}

impl StoragePrepGate {
    /// A gate parked on the network chooser: boot could not decide a network,
    /// so nothing may be prepared until the user picks one.
    pub(super) fn awaiting_network_choice() -> Self {
        Self::with_phase(BootPhase::AwaitingNetworkChoice)
    }

    /// A gate already raised for `network`. The caller spawns the preparation
    /// and hands the result channel over with [`Self::attach`].
    pub(super) fn preparing(network: Network) -> Self {
        Self::with_phase(BootPhase::Preparing { network })
    }

    fn with_phase(phase: BootPhase) -> Self {
        Self {
            phase,
            prepared: BTreeSet::new(),
            orphaned: false,
            pending: None,
            failure: None,
            overlay: None,
            auto_start_spv: false,
            arm_spv_block: false,
            #[cfg(feature = "testing")]
            test_hold: None,
        }
    }

    pub(super) fn phase(&self) -> BootPhase {
        self.phase
    }

    pub(super) fn arm_spv_block_on_success(&self) -> bool {
        self.arm_spv_block
    }

    /// Whether this network's storage was already prepared in this process.
    pub(super) fn is_prepared(&self, network: Network) -> bool {
        self.prepared.contains(&network)
    }

    /// Raise the gate for `network` and adopt the result channel of the
    /// preparation the caller just spawned. `auto_start_spv` is carried through
    /// to the [`GateEvent::Prepared`] that lifts it, so chain sync starts as a
    /// continuation of preparation rather than alongside it.
    pub(super) fn attach(
        &mut self,
        network: Network,
        auto_start_spv: bool,
        arm_spv_block: bool,
        result: tokio::sync::oneshot::Receiver<Result<(), TaskError>>,
    ) {
        self.phase = BootPhase::Preparing { network };
        self.auto_start_spv = auto_start_spv;
        self.arm_spv_block = arm_spv_block;
        self.failure = None;
        self.orphaned = false;
        self.pending = Some(PendingPrepare {
            network,
            started: Instant::now(),
            result,
            stuck: false,
        });
    }

    /// Test seam: raise the gate with no preparation behind it, so a kittest can
    /// drive the REAL frame loop against a gate that never completes and assert
    /// what renders above it. Mirrors `AppState::test_arm_spv_block`.
    #[cfg(feature = "testing")]
    pub(super) fn test_raise(&mut self, network: Network) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.test_hold = Some(tx);
        self.attach(network, false, false, rx);
    }

    /// Test seam: drop the in-flight preparation while leaving the gate raised,
    /// reproducing the state a reset-after-attach used to leave behind.
    #[cfg(feature = "testing")]
    pub(super) fn test_orphan(&mut self) {
        self.pending = None;
    }

    /// Test clock seam: shift the in-flight preparation's start into the past,
    /// so a kittest can cross the stuck threshold without waiting it out.
    #[cfg(feature = "testing")]
    pub(super) fn test_backdate_preparation(&mut self, by: std::time::Duration) {
        if let Some(pending) = self.pending.as_mut() {
            pending.started = pending.started.checked_sub(by).unwrap_or(pending.started);
        }
    }

    /// Test seam: resolve the raised gate's preparation as `error`, so a kittest
    /// can drive the REAL terminal-failure surface for an error the test picks.
    #[cfg(feature = "testing")]
    pub(super) fn test_fail(&mut self, error: TaskError) {
        if let Some(tx) = self.test_hold.take() {
            let _ = tx.send(Err(error));
        }
    }

    /// Test seam: resolve the raised gate's preparation successfully, so a
    /// kittest can drive the REAL terminal transition — screen construction,
    /// route re-resolution, chain-sync start — without waiting on real storage.
    #[cfg(feature = "testing")]
    pub(super) fn test_complete(&mut self) {
        if let Some(tx) = self.test_hold.take() {
            let _ = tx.send(Ok(()));
        }
    }

    /// Drive the gate for one frame: poll the in-flight preparation, keep the
    /// overlay's copy in step with the published progress, and drain the
    /// terminal surface's buttons.
    pub(super) fn update(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
        migration_state: &MigrationState,
    ) -> Option<GateEvent> {
        if !matches!(self.phase, BootPhase::Preparing { .. }) {
            self.overlay.take_and_clear();
            return None;
        }

        if let Some(event) = self.poll_pending() {
            return Some(event);
        }

        self.render(ctx, migration_state);
        if let Some(handle) = &self.overlay {
            match app_context.migration_status().wallet_load() {
                Some((current, total)) => handle.set_step(current, total),
                None => handle.clear_step(),
            };
        }
        self.drain_actions(app_context)
    }

    /// Resolve the in-flight preparation if it has finished.
    fn poll_pending(&mut self) -> Option<GateEvent> {
        let pending = self.pending.as_mut()?;
        let network = pending.network;
        match pending.result.try_recv() {
            Ok(Ok(())) => {
                self.pending = None;
                self.overlay.take_and_clear();
                self.prepared.insert(network);
                self.phase = BootPhase::Ready;
                tracing::info!(?network, "Storage preparation finished; releasing the app");
                Some(GateEvent::Prepared {
                    network,
                    start_spv: self.auto_start_spv,
                    arm_spv_block: self.arm_spv_block,
                })
            }
            Ok(Err(error)) => {
                self.pending = None;
                self.overlay.take_and_clear();
                tracing::error!(?network, error = %error, "Storage preparation failed");
                self.failure = Some(PrepareFailure {
                    network,
                    retryable: !matches!(
                        error,
                        TaskError::SavedDataTooOld { .. } | TaskError::SavedDataTooNew { .. }
                    ) && !crate::backend_task::is_terminal_storage_open_error(&error),
                    error,
                });
                None
            }
            // The preparation task was dropped without reporting — treat it as a
            // retryable failure rather than blocking forever on a dead channel.
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.pending = None;
                self.overlay.take_and_clear();
                tracing::error!(
                    ?network,
                    "Storage preparation ended without a result; offering a retry"
                );
                self.failure = Some(PrepareFailure {
                    network,
                    retryable: true,
                    error: TaskError::WalletStorageNotReady,
                });
                None
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
        }
    }

    /// Raise or update the overlay for this frame.
    fn render(&mut self, ctx: &egui::Context, migration_state: &MigrationState) {
        // A repaint every frame: the gate's only progress signal is a background
        // task, so egui would otherwise go idle and never poll it again.
        ctx.request_repaint();

        if let Some(failure) = &self.failure {
            let mut config = OverlayConfig::new();
            if failure.retryable {
                config = config
                    .with_action("Try again", STORAGE_PREP_RETRY_ACTION_ID)
                    .with_secondary_action("Close the app", STORAGE_PREP_CLOSE_ACTION_ID);
            } else {
                config = config.with_action("Close the app", STORAGE_PREP_CLOSE_ACTION_ID);
            }
            let message = if failure.retryable {
                STORAGE_PREP_FAILED_MESSAGE.to_string()
            } else {
                failure.error.to_string()
            };
            config = config
                .with_description(message)
                .with_keyboard_escape(STORAGE_PREP_CLOSE_ACTION_ID);
            if self.overlay.is_none() {
                self.overlay.raise(ctx, "", config);
            }
            return;
        }

        let stuck = self.mark_stuck_if_overdue(migration_state) || self.raise_if_orphaned();
        let description = if stuck {
            STORAGE_PREP_STUCK_MESSAGE
        } else {
            storage_prep_description(migration_state)
        };

        if let Some(handle) = &self.overlay {
            handle.set_description(description);
        } else {
            // Preparation cannot continue safely in the background. Only the stuck branch
            // adds an exit, and it re-raises to do so — an `OverlayHandle`'s
            // button methods append, so attaching per frame would stack copies.
            let mut config = OverlayConfig::new().with_description(description);
            if stuck {
                config = config
                    .with_action("Close the app", STORAGE_PREP_CLOSE_ACTION_ID)
                    .with_keyboard_escape(STORAGE_PREP_CLOSE_ACTION_ID);
            }
            self.overlay.raise(ctx, "", config);
        }
    }

    /// Whether the gate is up with nothing behind it, which no code path is
    /// allowed to produce.
    ///
    /// A raised gate with no preparation to poll and no failure to show is
    /// unreachable by design — every reset is followed by an attach or by
    /// `Ready` — but it is also the one gate bug the user cannot work around:
    /// no screen renders beneath it and, without this, no button appears on it.
    /// Treating it as stuck costs a wrong-ish sentence in a state that should
    /// never occur and buys an exit that is never missing.
    fn raise_if_orphaned(&mut self) -> bool {
        if self.pending.is_some() || !matches!(self.phase, BootPhase::Preparing { .. }) {
            return false;
        }
        if !self.orphaned {
            self.orphaned = true;
            tracing::error!(
                phase = ?self.phase,
                "Storage-preparation gate is raised with nothing to wait for; offering to close the app",
            );
            self.overlay.take_and_clear();
        }
        true
    }

    /// Whether preparation has been running long enough to surface the stuck
    /// copy. On the transition, logs once and lowers the overlay so the branch
    /// above re-raises it carrying the "Close the app" exit.
    ///
    /// The budget covers unattended work only. A password prompt is preparation
    /// waiting for the person at the keyboard, which it is designed to do for as
    /// long as it takes, so the clock restarts when they answer — and a prompt
    /// that outlives the budget never leaves the stuck copy latched behind it.
    fn mark_stuck_if_overdue(&mut self, migration_state: &MigrationState) -> bool {
        if MigrationReconciler::is_prompting(migration_state) {
            let latched = match self.pending.as_mut() {
                Some(pending) => {
                    pending.started = Instant::now();
                    std::mem::take(&mut pending.stuck)
                }
                None => false,
            };
            if latched {
                self.overlay.take_and_clear();
            }
            return false;
        }
        let Some(pending) = self.pending.as_mut() else {
            return false;
        };
        if pending.started.elapsed() < STORAGE_PREP_STUCK_TIMEOUT {
            return false;
        }
        if !pending.stuck {
            pending.stuck = true;
            tracing::warn!(
                network = ?pending.network,
                timeout_secs = STORAGE_PREP_STUCK_TIMEOUT.as_secs(),
                "Storage preparation exceeded its expected duration; offering to close the app",
            );
            self.overlay.take_and_clear();
        }
        true
    }

    /// Drain the overlay's own button clicks.
    fn drain_actions(&mut self, app_context: &Arc<AppContext>) -> Option<GateEvent> {
        let actions = self
            .overlay
            .as_ref()
            .map(|handle| handle.take_actions())
            .unwrap_or_default();
        let network = self
            .failure
            .as_ref()
            .map(|f| f.network)
            .or_else(|| self.pending.as_ref().map(|p| p.network))
            .unwrap_or(app_context.network);

        for action in actions {
            if action == STORAGE_PREP_CLOSE_ACTION_ID {
                return Some(GateEvent::Close);
            }
            if action == STORAGE_PREP_RETRY_ACTION_ID {
                self.failure = None;
                self.overlay.take_and_clear();
                // A retry re-runs the FULL sequence, not just the drain, so the
                // network must lose any claim to being prepared — otherwise a
                // later switch back would skip the gate on storage that never
                // finished preparing.
                self.prepared.remove(&network);
                return Some(GateEvent::Retry(network));
            }
        }
        None
    }

    /// Drop every surface and in-flight preparation belonging to the outgoing
    /// network, and settle the phase for `incoming`.
    ///
    /// The invariant, which is what a future change breaks without noticing:
    /// **this must run before the incoming network's `attach`, and every call
    /// must be followed by an attach or by [`BootPhase::Ready`] — a raised gate
    /// must always have something to wait for.** It clears `pending`, so the
    /// reverse order discards the very preparation the gate is waiting on and
    /// leaves it raised over nothing to poll, with no screen beneath it and no
    /// button on it.
    ///
    /// `prepared` is preserved: it is per-process knowledge that outlives any
    /// one network. A network already in it needs no preparation at all, so the
    /// phase goes straight to [`BootPhase::Ready`]; otherwise the caller's
    /// attach sets [`BootPhase::Preparing`].
    pub(super) fn reset_for_switch(&mut self, incoming: Network) {
        self.overlay.take_and_clear();
        self.pending = None;
        self.failure = None;
        if self.prepared.contains(&incoming) {
            self.phase = BootPhase::Ready;
        }
    }
}

/// The gate's own progress copy for the current published storage state.
///
/// Progress rides the one [`MigrationStatus`](crate::context::migration_status::MigrationStatus)
/// the banner already reads, so there is one step vocabulary rather than two.
/// States that publish no step (the sentinel short-circuit, a terminal outcome
/// reached before the frame loop caught up) fall back to the opening sentence
/// rather than leaving the card wordless.
fn storage_prep_description(state: &MigrationState) -> &'static str {
    match state {
        MigrationState::Running { step } => migration_running_text(*step),
        MigrationState::AwaitingWalletPasswords { .. } => STORAGE_PREP_PASSWORD_DESCRIPTION,
        _ => migration_running_text(crate::context::migration_status::MigrationStep::Wiring),
    }
}

/// Reconciles the data-migration banner and its wallet-password prompt.
pub(super) struct MigrationReconciler {
    /// Handle to the current migration banner, if displayed.
    banner_handle: Option<BannerHandle>,
    /// Wallet-storage error cleared once migration is terminal and its run guard is free.
    storage_startup_error: TransientBanner,
    /// Last-seen migration state so reconciliation fires only on change.
    last_state: Option<MigrationState>,
    /// Reused password-entry component for the current migrated wallet.
    wallet_unlock_popup: WalletUnlockPopup,
    /// Migrated wallet currently shown in the password prompt.
    prompt_wallet: Option<WalletSeedHash>,
}

impl MigrationReconciler {
    pub(super) fn new() -> Self {
        Self {
            banner_handle: None,
            storage_startup_error: TransientBanner::default(),
            last_state: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            prompt_wallet: None,
        }
    }

    /// Adopt a storage-not-ready error banner from the generic task fallback.
    pub(super) fn track_storage_startup_error(&mut self, handle: BannerHandle) {
        self.storage_startup_error.track(handle);
    }

    /// Clear banner and password-prompt state for a new network context.
    pub(super) fn reset_for_switch(&mut self) {
        if let Some(handle) = self.banner_handle.take() {
            handle.clear();
        }
        self.storage_startup_error.reset();
        self.last_state = None;
        self.wallet_unlock_popup.close();
        self.prompt_wallet = None;
    }

    /// Whether migration currently owns a blocking wallet-password prompt.
    pub(super) fn is_prompting(state: &MigrationState) -> bool {
        matches!(state, MigrationState::AwaitingWalletPasswords { .. })
    }

    /// Update the migration banner to reflect the current [`MigrationState`].
    /// Each step / outcome surfaces a single i18n-ready sentence. Retryable
    /// failures get a "Retry now" action button.
    ///
    /// `gate_raised` means the storage-preparation gate owns the frame. Its
    /// overlay already carries the in-progress copy — reading it from the same
    /// [`MigrationState`] — so the banner's in-progress arms stand down rather
    /// than printing the identical sentence twice. Terminal outcomes still
    /// surface: the gate lifts on those, and they carry the recovery actions.
    /// The password prompt is driven either way; it is what lets the gate finish.
    pub(super) fn update_banner(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
        frame_state: &MigrationState,
        gate_raised: bool,
    ) {
        let state = frame_state.clone();
        let storage_guard_resolved = !matches!(
            &state,
            MigrationState::Idle
                | MigrationState::Running { .. }
                | MigrationState::AwaitingWalletPasswords { .. }
        ) && app_context.try_lock_prepare_gate().is_ok();
        self.storage_startup_error.clear_if(storage_guard_resolved);
        self.update_password_prompt(ctx, app_context, &state);
        // The gate owns every surface while it is raised, including the failed
        // one: its overlay carries both the progress copy and the only "Try
        // again", so a banner here would either duplicate the sentence or offer
        // a second, competing retry that re-runs less than the gate's does.
        if gate_raised {
            return;
        }
        if self.last_state.as_ref() == Some(&state) {
            return;
        }
        self.last_state = Some(state.clone());

        // Clear the previous banner — text changes between steps must not stack.
        if let Some(handle) = self.banner_handle.take() {
            handle.clear();
        }

        match state {
            MigrationState::Idle | MigrationState::Ready => {}
            MigrationState::Running { step } => {
                let text = migration_running_text(step);
                let handle = MessageBanner::set_global(ctx, text, MessageType::Info);
                handle.with_elapsed();
                self.banner_handle = Some(handle);
            }
            MigrationState::AwaitingWalletPasswords { .. } => {
                let handle = MessageBanner::set_global(
                    ctx,
                    "Enter your wallet password to continue the storage update.",
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.banner_handle = Some(handle);
            }
            MigrationState::Success => {
                let handle = MessageBanner::set_global(
                    ctx,
                    "Storage update complete — your wallet is ready.",
                    MessageType::Success,
                );
                self.banner_handle = Some(handle);
            }
            MigrationState::SucceededWithUnreadableData {
                identities,
                top_ups,
            } => {
                let handle = MessageBanner::set_global(
                    ctx,
                    migration_unreadable_data_text(identities, top_ups),
                    MessageType::Warning,
                );
                handle.disable_auto_dismiss();
                let action = if identities > 0 && top_ups > 0 {
                    MIGRATION_UNREADABLE_ACK_ACTION_ID
                } else if identities > 0 {
                    MIGRATION_IDENTITIES_ACK_ACTION_ID
                } else {
                    MIGRATION_APP_DATA_ACK_ACTION_ID
                };
                handle.with_action("Got it", action);
                self.banner_handle = Some(handle);
            }
            MigrationState::FailedWithUnreadableIdentities { count, error } => {
                // Both DET-owned passes broke on the same launch. One Error banner
                // (retryable, sticky) names both problems — the identities need
                // reloading AND the app-data update must be retried — so neither
                // silently hides the other.
                let handle = MessageBanner::set_global(
                    ctx,
                    migration_failed_with_unreadable_identities_text(count),
                    MessageType::Error,
                );
                handle.disable_auto_dismiss();
                handle.with_details(error.as_ref());
                handle.with_action("Retry now", MIGRATION_RETRY_ACTION_ID);
                self.banner_handle = Some(handle);
            }
            MigrationState::Failed { error } => {
                // A backend-not-ready failure used to reset to Idle and wait for
                // the frame loop's readiness poll to re-dispatch. That poll is
                // gone — storage preparation wires the backend before the drain,
                // so the gate's own path cannot produce this — and the reset
                // would now strand the run at Idle with no banner and no retry.
                // It reaches the retryable banner below instead, which is a
                // recovery the user can actually reach. Still possible from the
                // `FinishUnwire` retry task and the MCP join, neither of which
                // wires first.
                let task_error = migration_task_error(Arc::clone(&error));
                let retryable = matches!(&task_error, TaskError::MigrationFailed { .. });
                let message = if retryable {
                    "Storage update could not complete. Your data is safe.".to_string()
                } else {
                    task_error.to_string()
                };
                let handle = MessageBanner::set_global(ctx, message, MessageType::Error);
                handle.disable_auto_dismiss();
                // The collapsed details panel + log line get the full typed
                // `MigrationError` chain rather than a lossy `to_string()`.
                handle.with_details(error.as_ref());
                if retryable {
                    handle.with_action("Retry now", MIGRATION_RETRY_ACTION_ID);
                }
                self.banner_handle = Some(handle);
            }
        }
    }

    fn update_password_prompt(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
        state: &MigrationState,
    ) {
        let MigrationState::AwaitingWalletPasswords { wallets } = state else {
            self.wallet_unlock_popup.close();
            self.prompt_wallet = None;
            return;
        };

        if let Some(seed_hash) = self.prompt_wallet {
            let still_locked = wallets.contains(&seed_hash)
                && app_context
                    .wallet_arc(&seed_hash)
                    .is_ok_and(|wallet| wallet_needs_unlock(&wallet));
            if !still_locked {
                self.wallet_unlock_popup.close();
                self.prompt_wallet = None;
            }
        }

        if self.prompt_wallet.is_none() {
            self.prompt_wallet = wallets.iter().copied().find(|seed_hash| {
                app_context
                    .wallet_arc(seed_hash)
                    .is_ok_and(|wallet| wallet_needs_unlock(&wallet))
            });
            if self.prompt_wallet.is_some() {
                self.wallet_unlock_popup.open();
            } else {
                app_context
                    .migration_status()
                    .notify_wallet_password_submitted();
                return;
            }
        }

        let Some(seed_hash) = self.prompt_wallet else {
            return;
        };
        let Ok(wallet) = app_context.wallet_arc(&seed_hash) else {
            app_context
                .migration_status()
                .notify_wallet_password_submitted();
            return;
        };
        match self
            .wallet_unlock_popup
            .show_for_migration(ctx, &wallet, app_context)
        {
            MigrationWalletUnlockResult::Unlocked => {
                self.prompt_wallet = None;
                app_context
                    .migration_status()
                    .notify_wallet_password_submitted();
            }
            MigrationWalletUnlockResult::Skipped => {
                self.prompt_wallet = None;
                app_context.migration_status().skip_wallet(seed_hash);
            }
            MigrationWalletUnlockResult::Pending => {}
        }
    }

    /// Dismiss the migration banner on Escape, unless the migration is still
    /// running (kept sticky so ongoing progress is not hidden).
    pub(super) fn handle_esc(&mut self, ctx: &egui::Context) {
        let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if !esc_pressed {
            return;
        }
        if self
            .last_state
            .as_ref()
            .is_some_and(MigrationState::is_executing)
        {
            return;
        }
        if let Some(handle) = self.banner_handle.take() {
            handle.clear();
        }
    }

    /// Drain pending banner-action clicks. Migration Retry clears `last_state`
    /// before re-dispatching `FinishUnwire`; unreadable-row acknowledgements
    /// clear the durable warning records their banners name.
    pub(super) fn drain_actions(
        &mut self,
        ctx: &egui::Context,
        network: Network,
    ) -> Option<BackendTask> {
        let mut task = None;
        while let Some(action_id) = MessageBanner::take_action(ctx) {
            if action_id == MIGRATION_RETRY_ACTION_ID {
                tracing::info!(
                    target = "migration::cold_start",
                    ?network,
                    "User clicked migration Retry — re-dispatching FinishUnwire",
                );
                // Reset the reconciler so the new run's Running banner
                // overwrites the stale Failed one.
                self.last_state = None;
                task = Some(BackendTask::MigrationTask(MigrationTask::FinishUnwire));
            } else if action_id == MIGRATION_APP_DATA_ACK_ACTION_ID {
                tracing::info!(
                    target = "migration::cold_start",
                    ?network,
                    "User acknowledged the unreadable app-data warning",
                );
                task = Some(BackendTask::MigrationTask(
                    MigrationTask::AcknowledgeUnreadableAppData,
                ));
            } else if action_id == MIGRATION_IDENTITIES_ACK_ACTION_ID {
                tracing::info!(
                    target = "migration::cold_start",
                    ?network,
                    "User acknowledged the unreadable-identity warning",
                );
                task = Some(BackendTask::MigrationTask(
                    MigrationTask::AcknowledgeUnreadableIdentities,
                ));
            } else if action_id == MIGRATION_UNREADABLE_ACK_ACTION_ID {
                tracing::info!(
                    target = "migration::cold_start",
                    ?network,
                    "User acknowledged the combined unreadable-data warning",
                );
                task = Some(BackendTask::MigrationTask(
                    MigrationTask::AcknowledgeUnreadableData,
                ));
            } else {
                tracing::warn!(
                    target = "ui::banner",
                    action_id = %action_id,
                    "Unknown banner action id — dropping",
                );
            }
        }
        task
    }
}

/// One transaction whose broadcast outcome was ambiguous.
struct PendingWatch {
    txid: Txid,
    /// When the watch was adopted, i.e. when the user was first told to wait.
    since: Instant,
    /// Whether the stale re-wording has already been applied.
    stale: bool,
}

/// Finishes the sentence the ambiguous-outcome banner starts.
///
/// A payment whose broadcast came back unverified leaves the user holding a
/// "wait, then refresh" message and no way to learn the answer except by
/// checking by hand. This adopts that banner and watches the wallet's own
/// display snapshot for the transaction to reach an InstantSend lock or a mined
/// block, then replaces it with a plain confirmation.
///
/// Lives above the screens on purpose: the watch has to survive the user
/// navigating away from Send, which screen state does not.
///
/// It owns the two banners rather than each watch holding its own, because
/// [`MessageBanner`] keys banners by exact text: every ambiguous outcome —
/// each watch's, plus every outcome that arrived without a transaction id —
/// is one and the same banner. Retiring it is therefore a decision about all
/// of them at once, taken in [`Self::sync_banners`].
pub(super) struct PendingConfirmation {
    watches: Vec<PendingWatch>,
    /// The shared ambiguous-outcome banner, adopted from the error arm.
    ambiguous: Option<BannerHandle>,
    /// That banner's copy, kept so it can be raised again if the global list
    /// evicts it at capacity while the question it asks is still open. The
    /// text originates in the error arm, so there is nothing else to rebuild
    /// it from.
    ambiguous_text: Option<String>,
    /// Whether an ambiguous outcome arrived with no transaction id (identity
    /// registration, top-up, platform-address funding, asset locks). No watch
    /// can ever answer it, so it is held purely as a claim on the shared
    /// banner: another payment's verdict must not clear it away.
    unwatchable: bool,
    /// The shared stale banner, raised while any claim needs it.
    stale: Option<BannerHandle>,
    /// One confirmation banner per answered payment, each naming its own
    /// transaction. Not shared the way the two warnings are: those ask the
    /// same question of every claim at once, whereas a confirmation speaks for
    /// exactly one payment, and with several waiting the user has to be able
    /// to tell which one it means.
    ///
    /// Held, and raised without auto-dismiss, for the same reason the question
    /// is: this feature exists because the user walked away, so the answer has
    /// to still be on screen when they come back. A default `Success` banner
    /// retires itself after five seconds, which would take the answer away
    /// moments after [`Self::sync_banners`] retired the question.
    confirmed: Vec<BannerHandle>,
    /// Whether a watch was retired at [`MAX_PENDING_WATCHES`]. The watch is
    /// gone but its stale advice stays — the transaction is still out there.
    retired: bool,
    /// `None` until the first poll, so a watch adopted this frame is resolved
    /// on the next one rather than sitting out a full interval.
    last_poll: Option<Instant>,
}

impl PendingConfirmation {
    pub(super) fn new() -> Self {
        Self {
            watches: Vec::new(),
            ambiguous: None,
            ambiguous_text: None,
            unwatchable: false,
            stale: None,
            confirmed: Vec::new(),
            retired: false,
            last_poll: None,
        }
    }

    /// Adopt the banner raised for an ambiguous broadcast of `txid`, sent on
    /// `origin` while `active` is the network now selected. The handle must
    /// already have auto-dismiss disabled — a pending funds question must not
    /// time out on its own.
    ///
    /// A payment from any other network is adopted but not watched: the watch
    /// reads the active network's wallet snapshot, which has never heard of it,
    /// so it could only ever go stale and hand the user transaction-history
    /// advice pointing at the wrong wallet.
    pub(super) fn track(
        &mut self,
        ctx: &egui::Context,
        txid: Txid,
        origin: Option<Network>,
        active: Network,
        banner: BannerHandle,
    ) {
        if origin != Some(active) {
            tracing::warn!(
                %txid,
                ?origin,
                %active,
                "Not watching a payment with an unverified outcome: it was not sent on the network now selected",
            );
            self.track_unwatchable(banner);
            return;
        }
        self.adopt_ambiguous(banner);
        // Re-tracking the same transaction (the user retried, upstream refused
        // again) keeps the original wait start: the network has had that long.
        if !self.watches.iter().any(|w| w.txid == txid) {
            self.watches.push(PendingWatch {
                txid,
                since: Instant::now(),
                stale: false,
            });
            if self.watches.len() > MAX_PENDING_WATCHES {
                let evicted = self.watches.remove(0);
                tracing::warn!(
                    txid = %evicted.txid,
                    "Stopped watching the oldest unconfirmed payment: {MAX_PENDING_WATCHES} are already being watched",
                );
                // Retire it to the durable surface rather than going silent.
                self.retired = true;
            }
        }
        self.sync_banners(ctx);
    }

    /// Adopt an ambiguous-outcome banner nothing here can ever answer — the
    /// outcome carries no transaction id, or belongs to another network. Held
    /// only so a watched payment's verdict cannot retire the message with it.
    pub(super) fn track_unwatchable(&mut self, banner: BannerHandle) {
        self.adopt_ambiguous(banner);
        self.unwatchable = true;
    }

    /// Take over the shared ambiguous-outcome banner, recording its copy from
    /// the banner itself so a later eviction can be undone. A handle that is
    /// somehow already dead leaves the last known copy in place rather than
    /// erasing the only means of restoring the message.
    fn adopt_ambiguous(&mut self, banner: BannerHandle) {
        if let Some(text) = banner.text() {
            self.ambiguous_text = Some(text);
        }
        self.ambiguous = Some(banner);
    }

    /// Drop every watch (network switch): the new network's snapshot knows
    /// nothing about these transactions, so nothing here could ever resolve.
    pub(super) fn reset(&mut self) {
        self.watches.clear();
        self.unwatchable = false;
        self.retired = false;
        self.ambiguous.take_and_clear();
        self.ambiguous_text = None;
        self.stale.take_and_clear();
        // Each confirmation names a payment on the network being left, so it
        // would be read against the wrong wallet's history if it stayed.
        for banner in self.confirmed.drain(..) {
            banner.clear();
        }
    }

    /// Re-read the snapshot for every open watch, at most once per
    /// [`PENDING_POLL_INTERVAL`]. Cheap to call every frame: with no watch
    /// open it does nothing at all.
    pub(super) fn update(&mut self, ctx: &egui::Context, app_context: &Arc<AppContext>) {
        let throttled = self
            .last_poll
            .is_some_and(|last| last.elapsed() < PENDING_POLL_INTERVAL);
        // A claim with nothing to re-read still ticks: an outcome carrying no
        // transaction id, or a watch retired at the cap, owns a banner that
        // has to outlive an eviction just as a watched payment's does.
        let idle = self.watches.is_empty() && !self.unwatchable && !self.retired;
        if idle || throttled {
            return;
        }
        self.last_poll = Some(Instant::now());
        if self.watches.is_empty() {
            self.sync_banners(ctx);
            return;
        }
        let Ok(backend) = app_context.wallet_backend() else {
            // Backend not wired (boot, or mid network switch) — the snapshot it
            // publishes is what we read, so retry on a later tick. Reconcile the
            // banners first all the same: restoring an evicted warning needs no
            // snapshot, and these are the windows that flood the list with
            // startup and connection messages in the first place.
            self.sync_banners(ctx);
            return;
        };
        self.apply(ctx, |txid| backend.transaction_confirmation(txid));
    }

    /// Apply one tick's verdicts. Split from [`Self::update`] so the banner
    /// transitions can be driven against a synthetic snapshot.
    fn apply(
        &mut self,
        ctx: &egui::Context,
        confirmation: impl Fn(&Txid) -> Option<TransactionConfirmation>,
    ) {
        let mut open = Vec::with_capacity(self.watches.len());
        let mut confirmed = Vec::new();
        for mut watch in self.watches.drain(..) {
            match pending_step(
                confirmation(&watch.txid),
                watch.since.elapsed(),
                watch.stale,
            ) {
                PendingStep::Confirmed => {
                    tracing::info!(txid = %watch.txid, "A payment with an unverified outcome is confirmed on the network");
                    confirmed.push(watch.txid);
                }
                PendingStep::Stale => {
                    tracing::warn!(txid = %watch.txid, "A payment with an unverified outcome is still unconfirmed");
                    watch.stale = true;
                    open.push(watch);
                }
                PendingStep::Waiting => open.push(watch),
            }
        }
        self.watches = open;
        // Raise the answer first, reconcile after. Raising evicts the oldest
        // banner once the list is full, so doing it second could knock out the
        // warning `sync_banners` had just restored. Reconciling last makes the
        // warning the one that survives a full list — the message that guards
        // the user's money outranks the one that merely reassures them.
        for txid in confirmed {
            let mut banner = None;
            banner.raise_persistent(ctx, pending_confirmed_message(&txid), MessageType::Success);
            self.confirmed.extend(banner);
        }
        self.sync_banners(ctx);
    }

    /// Bring both shared banners in line with the claims still open. Each is
    /// keyed by its text, so one banner speaks for every claim of its kind: it
    /// may only be retired once nothing still speaks through it. A claim also
    /// re-raises its banner if the global list evicted it at capacity, so
    /// unrelated notifications cannot end a message about the user's money.
    fn sync_banners(&mut self, ctx: &egui::Context) {
        if !self.unwatchable && !self.watches.iter().any(|w| !w.stale) {
            self.ambiguous.take_and_clear();
            self.ambiguous_text = None;
        } else if self.ambiguous.was_evicted() {
            // Unrelated app chatter pushed the warning out while the user's
            // money is still unaccounted for. Restoring it loses only the
            // collapsible details; the sentence is what protects them. A
            // banner the user dismissed is not evicted, so it stays gone.
            if let Some(text) = self.ambiguous_text.clone() {
                self.ambiguous
                    .raise_persistent(ctx, text, MessageType::Error);
            }
        }
        if self.retired || self.watches.iter().any(|w| w.stale) {
            if self.stale.is_none() || self.stale.was_evicted() {
                self.stale
                    .raise_persistent(ctx, PENDING_STALE_MESSAGE, MessageType::Warning);
            }
        } else {
            self.stale.take_and_clear();
        }
    }

    /// Transactions currently being watched (test seam / observation).
    #[cfg(test)]
    pub(super) fn watched(&self) -> Vec<Txid> {
        self.watches.iter().map(|w| w.txid).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::hashes::Hash;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn test_app_context(dir: &std::path::Path) -> Arc<AppContext> {
        crate::app_dir::ensure_env_file(dir);
        let db = Arc::new(crate::database::Database::new(dir.join("data.db")).expect("db"));
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
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("AppContext")
    }

    #[test]
    fn full_resync_overlay_waits_for_matching_completion_despite_synced_state() {
        let tmp = tempfile::tempdir().unwrap();
        let app = test_app_context(tmp.path());
        let mut harness = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui(|ui| {
                crate::ui::components::ProgressOverlay::render_global(ui.ctx(), false);
            });
        let ctx = harness.ctx.clone();
        let request =
            crate::backend_task::BackendTaskContext::for_dispatch(&BackendTask::CoreTask(
                crate::backend_task::core::CoreTask::FullResyncWallet { seed_hash: [1; 32] },
            ));
        let other = crate::backend_task::BackendTaskContext::for_dispatch(&BackendTask::CoreTask(
            crate::backend_task::core::CoreTask::FullResyncWallet { seed_hash: [1; 32] },
        ));
        let mut block = SpvBlockReconciler::new(false);
        block.arm_resync(request.clone());
        block.update(&ctx, &app);
        assert!(block.is_overlaying());
        block.finish_resync(&other);
        assert!(block.armed);
        block.finish_resync(&request);
        assert!(!block.is_overlaying());
        assert!(!crate::ui::components::ProgressOverlay::has_global(&ctx));
        assert!(!block.armed);
        block.arm_resync(request.clone());
        block.update(&ctx, &app);
        harness.run_steps(3);
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
        assert!(block.update(&ctx, &app).is_none());
        block.update(&ctx, &app);
        harness.run_steps(3);
        harness.get_by_label("Stop syncing").click();
        harness.run_steps(3);
        assert_eq!(block.update(&ctx, &app), Some(AppAction::StopSpv));
        assert!(!block.is_overlaying());
        block.arm_resync(request);
        block.update(&ctx, &app);
        block.reset();
        assert!(!block.is_overlaying());
    }

    /// Publish `state`, let the reconciler build its banner, click the named
    /// button, and return whatever task the click routed to. Drives the real
    /// `update_banner` → render → click → `drain_actions` chain, so a banner that
    /// never wires its action fails here instead of passing a test that hand-rolls
    /// the wiring it was supposed to prove.
    fn click_banner_action(state: MigrationState, label: &str) -> Option<BackendTask> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        app_context.migration_status().set_state(state);

        let mut reconciler = MigrationReconciler::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 260.0))
            .build_ui(MessageBanner::show_global);

        let frame_state = app_context.migration_status().state();
        reconciler.update_banner(&harness.ctx, &app_context, frame_state.as_ref(), false);
        harness.run();
        harness.get_by_label(label).click();
        harness.run();

        reconciler.drain_actions(&harness.ctx, app_context.network)
    }

    /// The ambiguous-outcome copy the reconciler adopts, verbatim from
    /// `TaskError::TransactionConfirmationUnknown`.
    const AMBIGUOUS: &str = "Your transaction was sent but the confirmation could not be verified. Wait a moment, then refresh your balance before sending it again.";

    fn banner_harness() -> Harness<'static> {
        Harness::builder()
            .with_size(egui::vec2(900.0, 400.0))
            .build_ui(MessageBanner::show_global)
    }

    /// The network these tests run their payments on.
    const ACTIVE: Network = Network::Testnet;

    /// The countdown a banner on the default short (five-second) auto-dismiss
    /// renders in its first second of life. A persistent banner renders no
    /// countdown at all, so this string's absence is the observable difference
    /// between a message that will expire and one that will not.
    const COUNTDOWN_AT_FULL_TERM: &str = "(5s)";

    /// Raise a fresh ambiguous-outcome banner, exactly as the generic error arm
    /// in `AppState::update` does.
    fn ambiguous_banner(ctx: &egui::Context) -> BannerHandle {
        let banner = MessageBanner::set_global(ctx, AMBIGUOUS, MessageType::Error);
        banner.disable_auto_dismiss();
        banner
    }

    /// Adopt a fresh ambiguous-outcome banner for `txid`, sent on the network
    /// still selected.
    fn adopt(reconciler: &mut PendingConfirmation, ctx: &egui::Context, txid: Txid) {
        let banner = ambiguous_banner(ctx);
        reconciler.track(ctx, txid, Some(ACTIVE), ACTIVE, banner);
    }

    fn mined(height: u32) -> Option<TransactionConfirmation> {
        Some(TransactionConfirmation {
            status: crate::model::wallet::TransactionStatus::Confirmed,
            height: Some(height),
        })
    }

    /// The whole point of the feature: the user is told to wait, the network
    /// takes the payment, and the app says so without being asked.
    #[test]
    fn a_confirmed_watch_replaces_the_ambiguous_banner_with_a_confirmation() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let txid = Txid::from_byte_array([1u8; 32]);
        adopt(&mut reconciler, &harness.ctx, txid);
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_some());

        reconciler.apply(&harness.ctx, |_| mined(1_234));
        harness.run();

        assert!(
            harness.query_by_label(AMBIGUOUS).is_none(),
            "the stale wait-and-refresh advice must be retired once the outcome is known"
        );
        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&txid))
                .is_some()
        );
        assert!(
            reconciler.watched().is_empty(),
            "a resolved watch must stop costing a snapshot scan"
        );
    }

    /// The answer must outlast the question. `sync_banners` retires the
    /// ambiguous warning on the same tick the confirmation goes up, so if the
    /// confirmation carried the default five-second `Success` timer the user
    /// would come back to a blank screen — knowing less than before the watch
    /// existed, with the payment's fate again theirs to work out by hand.
    ///
    /// A banner that will expire renders a countdown next to its text; a
    /// persistent one renders none. Asserting on that annotation pins the
    /// timer's absence without having to make five seconds pass. The control
    /// case below fixes the meaning of the check: if the annotation's shape
    /// ever changes, it fails loudly here rather than passing vacuously.
    #[test]
    fn a_confirmation_does_not_expire_while_the_user_is_away() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let txid = Txid::from_byte_array([2u8; 32]);
        adopt(&mut reconciler, &harness.ctx, txid);

        reconciler.apply(&harness.ctx, |_| mined(1_234));
        harness.run();

        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&txid))
                .is_some()
        );
        assert!(
            harness.query_by_label(COUNTDOWN_AT_FULL_TERM).is_none(),
            "the confirmation must not carry an auto-dismiss timer: the whole \
             point is that the user is not watching when it arrives",
        );

        // Control: an ordinary Success banner does show that countdown, so the
        // assertion above is testing the timer and not a stale label string.
        MessageBanner::set_global(&harness.ctx, "an ordinary success", MessageType::Success);
        harness.run();
        assert!(
            harness.query_by_label(COUNTDOWN_AT_FULL_TERM).is_some(),
            "a default Success banner is expected to show its countdown",
        );
    }

    /// A transaction sitting in the local mempool is not a verdict, and the
    /// banner must not move on it.
    #[test]
    fn an_unresolved_watch_leaves_the_ambiguous_banner_alone() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let txid = Txid::from_byte_array([2u8; 32]);
        adopt(&mut reconciler, &harness.ctx, txid);

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(harness.query_by_label(AMBIGUOUS).is_some());
        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&txid))
                .is_none()
        );
        assert_eq!(reconciler.watched(), vec![txid]);
    }

    /// Two payments can be unverified at once, and every ambiguous banner is
    /// literally the same banner — the text is identical, and banners key by
    /// text. One payment's verdict must not retire the message the other
    /// payment's user is still waiting on.
    #[test]
    fn one_watchs_confirmation_leaves_the_other_watch_its_banner() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let first = Txid::from_byte_array([10u8; 32]);
        let second = Txid::from_byte_array([11u8; 32]);
        adopt(&mut reconciler, &harness.ctx, first);
        adopt(&mut reconciler, &harness.ctx, second);

        reconciler.apply(&harness.ctx, |txid| {
            (*txid == first).then(|| mined(9)).flatten()
        });
        harness.run();

        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&first))
                .is_some(),
            "the confirmation must name the payment that actually landed"
        );
        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&second))
                .is_none(),
            "and must not be readable as the payment still in the air"
        );
        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "the payment still unanswered must keep the message telling its user to wait"
        );
        assert_eq!(reconciler.watched(), vec![second]);
    }

    /// The stale re-wording speaks for one watch. Raising it must not take the
    /// wait-and-see message away from a sibling watch still inside its window.
    #[test]
    fn a_stale_watch_leaves_a_sibling_still_waiting_its_banner() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let old = Txid::from_byte_array([12u8; 32]);
        let fresh = Txid::from_byte_array([13u8; 32]);
        adopt(&mut reconciler, &harness.ctx, old);
        adopt(&mut reconciler, &harness.ctx, fresh);
        reconciler.watches[0].since = Instant::now()
            .checked_sub(PENDING_STALE_AFTER)
            .expect("the test clock predates the stale threshold");

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());
        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "the watch still inside its window must keep its own message"
        );
        assert_eq!(reconciler.watched(), vec![old, fresh]);
    }

    /// An ambiguous outcome with no transaction id (identity registration,
    /// top-ups, asset locks) shares the one ambiguous banner but can never be
    /// answered here. An unrelated payment confirming says nothing about it and
    /// must not take its message away.
    #[test]
    fn a_confirmation_leaves_an_outcome_with_no_transaction_id_alone() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let txid = Txid::from_byte_array([14u8; 32]);
        adopt(&mut reconciler, &harness.ctx, txid);
        reconciler.track_unwatchable(ambiguous_banner(&harness.ctx));

        reconciler.apply(&harness.ctx, |_| mined(21));
        harness.run();

        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&txid))
                .is_some()
        );
        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "an outcome no watch can answer must keep its message when another payment confirms"
        );
        assert!(reconciler.watched().is_empty());
    }

    /// A payment dispatched before a network switch can only be answered by the
    /// network it was sent on: the active network's snapshot has never heard of
    /// it, so watching it here would poll forever and then hand the user
    /// transaction-history advice for the wrong wallet. Its banner still
    /// stands — the outcome really is unknown.
    #[test]
    fn a_payment_that_did_not_come_from_the_active_network_is_not_watched() {
        for origin in [None, Some(Network::Mainnet)] {
            let mut harness = banner_harness();
            let mut reconciler = PendingConfirmation::new();
            let txid = Txid::from_byte_array([15u8; 32]);
            let banner = ambiguous_banner(&harness.ctx);

            reconciler.track(&harness.ctx, txid, origin, ACTIVE, banner);
            harness.run();

            assert!(
                reconciler.watched().is_empty(),
                "a payment sent on {origin:?} must not be watched against {ACTIVE:?}"
            );
            assert!(
                harness.query_by_label(AMBIGUOUS).is_some(),
                "the outcome is still unknown, so its message must stay"
            );
            assert!(
                harness.query_by_label(PENDING_STALE_MESSAGE).is_none(),
                "a watch that was never taken must not produce stale advice"
            );
        }
    }

    /// Past the threshold the original advice has expired, so the banner is
    /// re-worded once — and the watch stays open, so a late confirmation still
    /// resolves it instead of leaving the warning as the last word.
    #[test]
    fn a_stale_watch_is_re_worded_and_keeps_watching() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let txid = Txid::from_byte_array([3u8; 32]);
        adopt(&mut reconciler, &harness.ctx, txid);
        reconciler.watches[0].since = Instant::now()
            .checked_sub(PENDING_STALE_AFTER)
            .expect("the test clock predates the stale threshold");

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_none());
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());
        assert_eq!(reconciler.watched(), vec![txid]);

        reconciler.apply(&harness.ctx, |_| mined(7));
        harness.run();
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_none());
        assert!(
            harness
                .query_by_label(&pending_confirmed_message(&txid))
                .is_some()
        );
    }

    /// A run of ambiguous sends must not grow the watch list without bound,
    /// and the watch that is dropped must not just go quiet — it is handed the
    /// transaction-history advice on the way out.
    #[test]
    fn the_oldest_watch_is_retired_to_the_stale_copy_at_the_cap() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        let first = Txid::from_byte_array([0u8; 32]);
        for n in 0..=MAX_PENDING_WATCHES as u8 {
            adopt(
                &mut reconciler,
                &harness.ctx,
                Txid::from_byte_array([n; 32]),
            );
        }
        harness.run();

        assert_eq!(reconciler.watched().len(), MAX_PENDING_WATCHES);
        assert!(!reconciler.watched().contains(&first));
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());
    }

    /// Raise enough unrelated banners to push everything already showing out
    /// of the global list, exactly as ordinary app chatter (connection,
    /// migration, per-task success) would during a long wait.
    fn evict_showing_banners(ctx: &egui::Context) {
        for n in 0..crate::ui::components::message_banner::MAX_BANNERS {
            MessageBanner::set_global(ctx, format!("Unrelated banner {n}"), MessageType::Info);
        }
    }

    /// The banner is the only thing telling the user their money may be in
    /// flight. Ordinary app chatter pushing it out of the capacity-bound
    /// global list must not be how that warning ends.
    #[test]
    fn an_evicted_ambiguous_banner_comes_back_while_the_question_is_open() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([4u8; 32]),
        );
        evict_showing_banners(&harness.ctx);
        harness.run();
        assert!(
            harness.query_by_label(AMBIGUOUS).is_none(),
            "the flood must actually evict the banner, or this test proves nothing"
        );

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "the unverified-payment warning must return while its answer is still unknown"
        );
    }

    /// The same loss, one copy later: once a wait has gone on long enough to
    /// earn the transaction-history advice, an eviction must not retire it.
    #[test]
    fn an_evicted_stale_banner_comes_back_while_a_watch_is_stale() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([5u8; 32]),
        );
        reconciler.watches[0].since = Instant::now()
            .checked_sub(PENDING_STALE_AFTER)
            .expect("the test clock predates the stale threshold");
        reconciler.apply(&harness.ctx, |_| None);
        harness.run();
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());

        evict_showing_banners(&harness.ctx);
        harness.run();
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_none());

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(
            harness.query_by_label(PENDING_STALE_MESSAGE).is_some(),
            "the stale advice must return while the transaction is still out there"
        );
    }

    /// The mixed verdict: one payment has waited long enough for the stale
    /// copy while another is still fresh, so both banners are owed at once. A
    /// flood takes both, and both must come back — restoring one and leaving
    /// the other would tell a half-truth about the user's money.
    #[test]
    fn both_banners_come_back_after_an_eviction_on_a_mixed_verdict() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([6u8; 32]),
        );
        reconciler.watches[0].since = Instant::now()
            .checked_sub(PENDING_STALE_AFTER)
            .expect("the test clock predates the stale threshold");
        reconciler.apply(&harness.ctx, |_| None);
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([7u8; 32]),
        );
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_some());
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());

        evict_showing_banners(&harness.ctx);
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_none());
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_none());

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "the fresh payment's warning must return"
        );
        assert!(
            harness.query_by_label(PENDING_STALE_MESSAGE).is_some(),
            "the long wait's advice must return alongside it"
        );
    }

    /// The other half of the restore rule. Coming back after an eviction must
    /// not turn into coming back after the user closed it — that would make an
    /// unavoidable nag out of a warning they have already read and acted on.
    #[test]
    fn a_dismissed_ambiguous_banner_is_not_raised_again() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([8u8; 32]),
        );
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_some());

        // What the dismiss button does: drop the banner, leaving the handle.
        reconciler
            .ambiguous
            .clone()
            .expect("the banner was adopted")
            .clear();
        harness.run();

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(
            harness.query_by_label(AMBIGUOUS).is_none(),
            "a warning the user closed must stay closed"
        );
    }

    /// Same rule for the later copy: dismissing the stale advice retires it,
    /// even though the watch it speaks for is still open.
    #[test]
    fn a_dismissed_stale_banner_is_not_raised_again() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([9u8; 32]),
        );
        reconciler.watches[0].since = Instant::now()
            .checked_sub(PENDING_STALE_AFTER)
            .expect("the test clock predates the stale threshold");
        reconciler.apply(&harness.ctx, |_| None);
        harness.run();
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());

        reconciler
            .stale
            .clone()
            .expect("the stale banner was raised")
            .clear();
        harness.run();

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();

        assert!(
            harness.query_by_label(PENDING_STALE_MESSAGE).is_none(),
            "advice the user closed must stay closed"
        );
    }

    /// An outcome with no transaction id has no watch to poll, so nothing was
    /// ticking to notice its banner had been evicted. That claim is exactly the
    /// one no verdict can ever answer, which makes losing its banner permanent.
    #[test]
    fn an_evicted_unwatchable_banner_comes_back_with_no_watch_to_poll() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        reconciler.track_unwatchable(ambiguous_banner(&harness.ctx));
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_some());
        assert!(
            reconciler.watched().is_empty(),
            "this claim has nothing to watch — that is the point"
        );

        evict_showing_banners(&harness.ctx);
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_none());

        reconciler.update(&harness.ctx, &app_context);
        harness.run();

        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "a claim nothing can answer must still keep its banner"
        );
    }

    /// `AppState::update` drains every queued task result before the
    /// reconciler ticks, and the poll throttle can suppress a tick for
    /// seconds more. A long burst of unrelated banners in that window must
    /// not cost the warning its right to come back.
    #[test]
    fn an_evicted_banner_comes_back_after_a_long_burst_before_the_next_tick() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        reconciler.track_unwatchable(ambiguous_banner(&harness.ctx));
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_some());

        // Many times the capacity, all before the reconciler gets to look.
        for n in 0..crate::ui::components::message_banner::MAX_BANNERS * 20 {
            MessageBanner::set_global(
                &harness.ctx,
                format!("Unrelated banner {n}"),
                MessageType::Info,
            );
        }
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_none());

        reconciler.update(&harness.ctx, &app_context);
        harness.run();

        assert!(
            harness.query_by_label(AMBIGUOUS).is_some(),
            "the warning must survive a burst of chatter, however long"
        );
    }

    /// The transactions belong to the network the user just left; their
    /// banners must go with them.
    #[test]
    fn reset_drops_every_watch_and_its_banner() {
        let mut harness = banner_harness();
        let mut reconciler = PendingConfirmation::new();
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([4u8; 32]),
        );
        harness.run();

        reconciler.reset();
        harness.run();

        assert!(reconciler.watched().is_empty());
        assert!(harness.query_by_label(AMBIGUOUS).is_none());
    }

    #[test]
    fn too_old_data_banner_shows_step_upgrade_without_retry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        let mut reconciler = MigrationReconciler::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 260.0))
            .build_ui(MessageBanner::show_global);
        let message = "This saved data was created by a much older version of Dash Evo Tool and can't be upgraded directly. Please install Dash Evo Tool 0.9.3 first and open your data with it once, then upgrade to this version.";
        let state = MigrationState::Failed {
            error: Arc::new(
                crate::backend_task::migration::MigrationError::LegacyDataTooOld {
                    found: 10,
                    minimum_supported: 11,
                },
            ),
        };

        reconciler.update_banner(&harness.ctx, &app_context, &state, false);
        harness.run();

        assert!(harness.query_by_label(message).is_some());
        assert!(harness.query_by_label("Retry now").is_none());
    }

    /// The unreadable-identity warning is acknowledgeable. It used to render as a
    /// sticky banner with NO action button at all: the user was told their signing
    /// keys had not come across and given no way to say "I understand", so the
    /// warning returned on every launch with no gesture that could retire it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_identities_banner_acknowledgement_routes_to_its_task() {
        let task = click_banner_action(
            MigrationState::SucceededWithUnreadableData {
                identities: 2,
                top_ups: 0,
            },
            "Got it",
        );
        assert_eq!(
            task,
            Some(BackendTask::MigrationTask(
                MigrationTask::AcknowledgeUnreadableIdentities
            )),
            "the identity warning must offer an acknowledgement that reaches its backend task",
        );
    }

    /// The combined banner names both problems in one message, so its single
    /// acknowledgement must retire BOTH records. Routing it to either single-signal
    /// task would leave the other half to re-raise on the next launch — a notice the
    /// user has already read and acted on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn combined_unreadable_banner_acknowledgement_routes_to_the_combined_task() {
        let task = click_banner_action(
            MigrationState::SucceededWithUnreadableData {
                identities: 2,
                top_ups: 3,
            },
            "Got it",
        );
        assert_eq!(
            task,
            Some(BackendTask::MigrationTask(
                MigrationTask::AcknowledgeUnreadableData
            )),
            "one banner naming both problems must retire both warnings on one click",
        );
    }

    /// The top-up warning keeps its own acknowledgement — the identity work above
    /// must not have re-routed the sibling banner to the wrong task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_top_ups_banner_acknowledgement_routes_to_its_task() {
        let task = click_banner_action(
            MigrationState::SucceededWithUnreadableData {
                identities: 0,
                top_ups: 2,
            },
            "Got it",
        );
        assert_eq!(
            task,
            Some(BackendTask::MigrationTask(
                MigrationTask::AcknowledgeUnreadableAppData
            )),
        );
    }

    /// Disconnected stays hidden throughout onboarding and appears on the first
    /// frame after onboarding ends, even when the connection state is unchanged.
    #[test]
    fn connection_banner_suppresses_disconnected_until_onboarding_ends() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        // ConnectionStatus defaults to Disconnected — no sync has been asked for.
        assert_eq!(
            app_context.connection_status().overall_state(),
            OverallConnectionState::Disconnected
        );
        let ctx = egui::Context::default();
        let mut banner = ConnectionBanner::new();

        // Frame 1: onboarding active, Disconnected — suppressed.
        assert!(banner.update(&ctx, &app_context, false, true).is_none());
        assert!(
            banner.handle.is_none(),
            "the Disconnected banner must stay hidden while onboarding is active"
        );

        // Frame 2: onboarding still active, state unchanged — still suppressed.
        assert!(banner.update(&ctx, &app_context, false, true).is_none());
        assert!(
            banner.handle.is_none(),
            "Disconnected must stay hidden for every frame while onboarding is active"
        );

        // Frame 3: onboarding ends, connection is still Disconnected — the real
        // banner must now appear even though `current_state` never changed.
        assert!(banner.update(&ctx, &app_context, false, false).is_none());
        assert!(
            banner.handle.is_some(),
            "a genuine Disconnected state must be reported once onboarding ends, \
             even if the connection state itself never changed"
        );
    }

    #[test]
    fn connection_banner_clears_quorum_startup_error_when_masternodes_become_ready() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        let message = "The network is still syncing. Please wait a moment and try again.";
        let mut reconciler = ConnectionBanner::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 260.0))
            .build_ui(MessageBanner::show_global);

        let handle = MessageBanner::set_global(&harness.ctx, message, MessageType::Error);
        handle.disable_auto_dismiss();
        reconciler.track_quorum_startup_error(handle);
        reconciler.update(&harness.ctx, &app_context, false, false);
        harness.run();
        assert!(harness.query_by_label(message).is_some());

        app_context.connection_status().set_masternodes_ready(true);
        reconciler.update(&harness.ctx, &app_context, false, false);
        harness.run();
        assert!(
            harness.query_by_label(message).is_none(),
            "the startup error must clear when quorum keys become available",
        );
    }

    #[test]
    fn migration_reconciler_waits_for_storage_guard_before_clearing_startup_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        let message = TaskError::WalletStorageNotReady.to_string();
        let mut reconciler = MigrationReconciler::new();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 260.0))
            .build_ui(MessageBanner::show_global);

        app_context
            .migration_status()
            .set_state(MigrationState::Running {
                step: crate::context::migration_status::MigrationStep::Detecting,
            });
        let handle = MessageBanner::set_global(&harness.ctx, &message, MessageType::Error);
        handle.disable_auto_dismiss();
        reconciler.track_storage_startup_error(handle);
        let state = app_context.migration_status().state();
        reconciler.update_banner(&harness.ctx, &app_context, state.as_ref(), false);
        harness.run();
        assert!(harness.query_by_label(&message).is_some());

        let migration_guard = app_context
            .try_lock_prepare_gate()
            .expect("migration guard");
        app_context
            .migration_status()
            .set_state(MigrationState::Ready);
        let state = app_context.migration_status().state();
        reconciler.update_banner(&harness.ctx, &app_context, state.as_ref(), false);
        harness.run();
        assert!(
            harness.query_by_label(&message).is_some(),
            "terminal state must not clear the startup error while storage remains locked",
        );

        drop(migration_guard);
        reconciler.update_banner(&harness.ctx, &app_context, state.as_ref(), false);
        harness.run();
        assert!(
            harness.query_by_label(&message).is_none(),
            "the startup error must clear when the storage update is ready and unlocked",
        );
    }

    #[test]
    fn connection_banner_restores_recurring_error_after_onboarding_suppression() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_context = test_app_context(tmp.path());
        let connection_status = app_context.connection_status();
        let ctx = egui::Context::default();
        let mut banner = ConnectionBanner::new();

        connection_status.set_spv_status(crate::model::spv_status::SpvStatus::Error);
        connection_status.refresh_state();
        assert!(banner.update(&ctx, &app_context, false, true).is_none());
        assert!(
            banner.handle.is_some(),
            "the first SPV error must be reported"
        );

        connection_status.set_spv_status(crate::model::spv_status::SpvStatus::Idle);
        connection_status.refresh_state();
        assert!(banner.update(&ctx, &app_context, false, true).is_none());
        assert!(
            banner.handle.is_none(),
            "Disconnected must be suppressed while onboarding is active"
        );

        connection_status.set_spv_status(crate::model::spv_status::SpvStatus::Error);
        connection_status.refresh_state();
        assert!(banner.update(&ctx, &app_context, false, true).is_none());
        assert!(
            banner.handle.is_some(),
            "a recurring SPV error must be restored after Disconnected suppression"
        );
    }
}

#[cfg(test)]
mod pending_confirmation_tests {
    use super::*;
    use dash_sdk::dpp::dashcore::hashes::Hash;

    fn seen(status: TransactionStatus, height: Option<u32>) -> Option<TransactionConfirmation> {
        Some(TransactionConfirmation { status, height })
    }

    /// The window the whole feature lives in: the transaction is in the
    /// snapshot because this app injected it locally, which says nothing about
    /// whether any peer took it. Keep the ambiguous banner, keep watching.
    #[test]
    fn a_mempool_only_transaction_keeps_waiting() {
        assert_eq!(
            pending_step(
                seen(TransactionStatus::Unconfirmed, None),
                Duration::from_secs(5),
                false
            ),
            PendingStep::Waiting,
        );
    }

    /// A transaction no loaded wallet has seen at all is likewise no verdict.
    #[test]
    fn an_unseen_transaction_keeps_waiting() {
        assert_eq!(
            pending_step(None, Duration::from_secs(5), false),
            PendingStep::Waiting,
        );
    }

    /// An InstantSend lock is driven only by a real lock message from the
    /// network, and arrives before any block — so it confirms on its own, with
    /// no height to wait for.
    #[test]
    fn an_instant_send_lock_confirms_without_a_height() {
        assert_eq!(
            pending_step(
                seen(TransactionStatus::InstantSendLocked, None),
                Duration::from_secs(5),
                false
            ),
            PendingStep::Confirmed,
        );
    }

    /// A mined transaction carries the height of the block holding it.
    #[test]
    fn a_mined_transaction_confirms() {
        for status in [TransactionStatus::Confirmed, TransactionStatus::ChainLocked] {
            assert_eq!(
                pending_step(seen(status, Some(1_234)), Duration::from_secs(5), false),
                PendingStep::Confirmed,
                "{status:?} at a known height must confirm"
            );
        }
    }

    /// The phantom guard. A record reported at a mined tier but carrying no
    /// block height is not evidence of a block, and this verdict becomes a
    /// message about someone's money — so withhold it and keep watching.
    #[test]
    fn a_mined_tier_without_a_height_keeps_waiting() {
        for status in [TransactionStatus::Confirmed, TransactionStatus::ChainLocked] {
            assert_eq!(
                pending_step(seen(status, None), Duration::from_secs(5), false),
                PendingStep::Waiting,
                "{status:?} without a height must not be treated as mined"
            );
        }
    }

    /// Past the threshold, "wait a moment, then refresh" has expired and the
    /// user needs different advice.
    #[test]
    fn a_long_unconfirmed_watch_goes_stale() {
        assert_eq!(
            pending_step(
                seen(TransactionStatus::Unconfirmed, None),
                PENDING_STALE_AFTER,
                false
            ),
            PendingStep::Stale,
        );
    }

    /// The re-wording happens once. Repeating it every tick would keep
    /// rebuilding a banner the user is trying to read, or dismiss.
    #[test]
    fn an_already_stale_watch_does_not_re_word_itself() {
        assert_eq!(
            pending_step(
                seen(TransactionStatus::Unconfirmed, None),
                PENDING_STALE_AFTER * 4,
                true
            ),
            PendingStep::Waiting,
        );
    }

    /// The watch never expires: a confirmation long after the copy changed
    /// still resolves it, so a slow network ends in the truth rather than in
    /// the warning.
    #[test]
    fn a_late_confirmation_still_beats_a_stale_watch() {
        assert_eq!(
            pending_step(
                seen(TransactionStatus::Confirmed, Some(9_001)),
                PENDING_STALE_AFTER * 4,
                true
            ),
            PendingStep::Confirmed,
        );
    }

    /// Both pending messages are written for the Everyday User: no jargon, and
    /// no promise about the funds becoming spendable again, which this app
    /// cannot verify.
    #[test]
    fn pending_copy_stays_jargon_free_and_promises_nothing_about_the_funds() {
        let confirmed = pending_confirmed_message(&Txid::from_byte_array([0xab; 32]));
        for message in [confirmed.as_str(), PENDING_STALE_MESSAGE] {
            for jargon in [
                "mempool",
                "broadcast",
                "InstantSend",
                "SPV",
                "nonce",
                "state transition",
                "txid",
                "UTXO",
            ] {
                assert!(
                    !message.contains(jargon),
                    "Expected no jargon ({jargon}) in: {message}"
                );
            }
            assert!(
                !message.contains("safe"),
                "the copy must not vouch for the safety of a resend: {message}"
            );
        }
    }

    /// The confirmation has to name the payment it answers for. Several can be
    /// waiting at once and the others keep their warning on screen beside it,
    /// so an unattributed "your transaction is confirmed" leaves the user to
    /// guess which one landed — and either guess ends in paying twice.
    #[test]
    fn the_confirmation_copy_names_the_payment_it_answers_for() {
        let first = Txid::from_byte_array([0x11; 32]);
        let second = Txid::from_byte_array([0x22; 32]);

        let message = pending_confirmed_message(&first);
        assert_ne!(
            message,
            pending_confirmed_message(&second),
            "two payments must not produce the same confirmation, or the shared \
             banner list collapses them into one indistinguishable message"
        );

        let full = first.to_string();
        let head = &full[..5];
        assert!(
            message.contains(head),
            "the copy must carry enough of the id to match the transaction \
             history row it sends the user to: {message}"
        );
    }

    /// The stale copy is reached for every non-confirmed observation, including
    /// one no loaded wallet has seen at all and one reported at a mined tier
    /// with no block height. Naming a status asserts a transaction-history row
    /// the app has not established — and, in the mined-tier case, one the
    /// wallet would render under a different word entirely.
    #[test]
    fn the_stale_copy_names_no_transaction_status_it_cannot_establish() {
        for status in ["Pending", "Unconfirmed", "Confirmed", "ChainLocked"] {
            assert!(
                !PENDING_STALE_MESSAGE.contains(status),
                "the copy must not name the {status} status: {PENDING_STALE_MESSAGE}"
            );
        }
        assert!(
            PENDING_STALE_MESSAGE.contains("transaction history"),
            "the copy must still hand the user the durable surface: {PENDING_STALE_MESSAGE}"
        );
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
    /// every state. This is the regression guard: a mid-session reconnect or
    /// per-block Synced→Syncing flip must not hard-block.
    #[test]
    fn unarmed_never_blocks() {
        for state in ALL_STATES {
            assert_eq!(
                spv_block_step(false, state),
                SpvBlockStep::Idle,
                "un-armed {state:?} must not block"
            );
        }
    }

    /// Armed + getting-connected (Connecting/Syncing/Disconnected) → hard block.
    ///
    /// There is no longer a "stand down without disarming" row: the background
    /// escape it modelled is gone. Cancel stops sync outright (after a confirm),
    /// which lands on Disarm through the reconciler, not on a third step.
    #[test]
    fn armed_blocks_while_getting_connected() {
        for state in [
            OverallConnectionState::Disconnected,
            OverallConnectionState::Connecting,
            OverallConnectionState::Syncing,
        ] {
            assert_eq!(spv_block_step(true, state), SpvBlockStep::Block);
        }
    }

    /// C1 / F-SPV-A — armed + terminal (Synced/Error) → Disarm: lower and disarm
    /// so ambient sync afterwards never re-blocks.
    #[test]
    fn armed_terminal_state_disarms() {
        for state in [
            OverallConnectionState::Synced,
            OverallConnectionState::Error,
        ] {
            assert_eq!(spv_block_step(true, state), SpvBlockStep::Disarm);
        }
    }

    /// The cancel action ids are stable — production raises them and the
    /// SPV-sync block reconciler matches on them; a typo would drop the click,
    /// and on the confirm pair it could drop it onto the wrong button.
    #[test]
    fn cancel_action_ids_are_stable() {
        assert_eq!(SPV_CANCEL_ACTION_ID, "spv:sync:cancel");
        assert_eq!(SPV_CANCEL_CONFIRM_ACTION_ID, "spv:sync:cancel_confirm");
        assert_eq!(SPV_CANCEL_KEEP_ACTION_ID, "spv:sync:cancel_keep");
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
mod policy_copy_tests {
    use super::*;
    /// Scheduled votes are never imported or inspected by the storage update:
    /// no migration copy may claim otherwise.
    #[test]
    fn migration_copy_does_not_mention_scheduled_votes() {
        let progress = MigrationStep::ALL.map(|step| migration_running_text(step).to_owned());
        let warnings = [(0, 0), (1, 0), (0, 2), (1, 2)]
            .map(|(identities, top_ups)| migration_unreadable_data_text(identities, top_ups));
        for text in progress.iter().chain(&warnings) {
            assert!(!text.to_lowercase().contains("scheduled vote"), "{text}");
        }
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
        let text = migration_unreadable_data_text(0, 2);
        assert!(text.contains("balance history"));
        assert!(text.contains("before adding more funds"));
        assert!(text.ends_with('.'));
    }
    /// TC-MIG-014 — every `MigrationStep` exposes a non-empty,
    /// sentence-shaped label so i18n extraction picks it up as one
    /// translation unit (no concatenation).
    #[test]
    fn migration_running_text_is_sentence_for_every_step() {
        for step in MigrationStep::ALL {
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
        let labels = MigrationStep::ALL.map(migration_running_text);
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
            MIGRATION_APP_DATA_ACK_ACTION_ID,
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
    /// The gate's copy must be self-contained, actionable Everyday-User text:
    /// what happened plus what the user can do, in complete sentences so i18n
    /// extracts each as one unit. No jargon — these surface behind a block the
    /// user cannot dismiss, so a vague sentence has no screen to fall back to.
    #[test]
    fn storage_prep_copy_is_actionable_and_jargon_free() {
        for text in [
            STORAGE_PREP_FAILED_MESSAGE,
            STORAGE_PREP_STUCK_MESSAGE,
            STORAGE_PREP_PASSWORD_DESCRIPTION,
            SPV_CANCEL_QUESTION,
        ] {
            assert!(
                text.ends_with('.') || text.ends_with('?'),
                "`{text}` is not a sentence"
            );
            for jargon in [
                "SPV",
                "backend",
                "wiring",
                "migration",
                "sentinel",
                "persister",
            ] {
                assert!(
                    !text.contains(jargon),
                    "`{text}` leaks the internal term `{jargon}`",
                );
            }
        }
        assert!(
            STORAGE_PREP_FAILED_MESSAGE.contains("unchanged"),
            "a failed update must say the user's wallets and keys are untouched",
        );
    }
    /// Every gate and cancel action id is distinct, so a click can never be
    /// attributed to the wrong button — the confirm/keep pair especially, where
    /// a collision would turn "Keep syncing" into a disconnect.
    #[test]
    fn overlay_action_ids_are_distinct() {
        let ids = [
            SPV_CANCEL_ACTION_ID,
            SPV_CANCEL_CONFIRM_ACTION_ID,
            SPV_CANCEL_KEEP_ACTION_ID,
            STORAGE_PREP_RETRY_ACTION_ID,
            STORAGE_PREP_CLOSE_ACTION_ID,
        ];
        let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len(), "duplicate overlay action id");
    }
}
