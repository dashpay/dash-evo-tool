//! Per-frame reconcilers extracted from [`AppState`](super::AppState).
//!
//! Each reconciler owns the fields it needs and exposes an `update()`/action
//! method the frame loop calls once per frame, mirroring the `ThemeState`
//! precedent. Reconcilers that must dispatch async work return the
//! [`BackendTask`] for `AppState` to run through its channel, keeping the
//! dispatch chokepoint in one place.
//!
//! The pure decision helpers and copy constants stay in [`super`]; a child
//! module can read its parent's private items, so no visibility widening is
//! needed.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use dash_sdk::dpp::dashcore::Network;
use eframe::egui;

use crate::backend_task::error::TaskError;
use crate::backend_task::migration::{MigrationTask, migration_task_error};
use crate::backend_task::{BackendTask, platform_info};
use crate::context::AppContext;
use crate::context::connection_status::{
    OverallConnectionState, SPV_SYNC_PHASE_COUNT, spv_phase_step, spv_progress_token,
};
use crate::context::migration_status::MigrationState;
use crate::model::wallet::WalletSeedHash;
use crate::ui::MessageType;
use crate::ui::components::wallet_unlock_popup::{
    MigrationWalletUnlockResult, WalletUnlockPopup, wallet_needs_unlock,
};
use crate::ui::components::{
    BannerHandle, MessageBanner, OptionOverlayExt, OverlayConfig, OverlayHandle,
};

use super::{
    AppAction, BootPhase, MIGRATION_IDENTITIES_ACK_ACTION_ID, MIGRATION_RETRY_ACTION_ID,
    MIGRATION_UNREADABLE_ACK_ACTION_ID, MIGRATION_VOTES_ACK_ACTION_ID, SPV_CANCEL_ACTION_ID,
    SPV_CANCEL_CONFIRM_ACTION_ID, SPV_CANCEL_KEEP_ACTION_ID, SPV_CANCEL_QUESTION,
    SPV_CONNECTING_DESCRIPTION, SPV_SYNCING_DESCRIPTION, STORAGE_PREP_CLOSE_ACTION_ID,
    STORAGE_PREP_FAILED_MESSAGE, STORAGE_PREP_PASSWORD_DESCRIPTION, STORAGE_PREP_RETRY_ACTION_ID,
    STORAGE_PREP_STUCK_MESSAGE, STORAGE_PREP_STUCK_TIMEOUT, SpvBlockStep,
    migration_failed_with_unreadable_identities_text, migration_running_text,
    migration_unreadable_data_text, spv_block_step,
};

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
    /// Whether the block is showing the Cancel confirmation rather than progress.
    confirming_cancel: bool,
}

impl SpvBlockReconciler {
    pub(super) fn new(armed: bool) -> Self {
        Self {
            overlay: None,
            armed,
            confirming_cancel: false,
        }
    }

    /// Arm a fresh user-initiated episode (boot auto-start, Connect button,
    /// post-onboarding auto-start).
    pub(super) fn arm(&mut self) {
        self.armed = true;
        self.confirming_cancel = false;
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
        self.overlay = None;
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
        match spv_block_step(self.armed, state) {
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
    Prepared { network: Network, start_spv: bool },
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
            #[cfg(feature = "testing")]
            test_hold: None,
        }
    }

    pub(super) fn phase(&self) -> BootPhase {
        self.phase
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
        result: tokio::sync::oneshot::Receiver<Result<(), TaskError>>,
    ) {
        self.phase = BootPhase::Preparing { network };
        self.auto_start_spv = auto_start_spv;
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
        self.attach(network, false, rx);
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
                    ),
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
            // No escape while preparation is healthy: D2 replaces "continue in
            // the background" with no background at all. Only the stuck branch
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
    /// Call this *before* attaching `incoming`'s preparation — it clears
    /// `pending`, so the reverse order discards the very preparation the gate is
    /// waiting on and leaves it raised over nothing to poll. A network this
    /// process has already prepared needs no preparation at all, so the phase
    /// goes straight to [`BootPhase::Ready`]; otherwise the caller's attach sets
    /// [`BootPhase::Preparing`].
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

/// Reconciles the data-migration banner and drives the cold-start
/// `FinishUnwire` dispatch per network.
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

    /// Clear the migration banner and force re-evaluation (network switch). The
    /// per-network dispatch guard is intentionally NOT reset — it is scoped per
    /// network so a return to a seen network never re-drains.
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
        ) && app_context.prepare_gate.try_lock().is_ok();
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
                votes,
                top_ups,
            } => {
                let handle = MessageBanner::set_global(
                    ctx,
                    migration_unreadable_data_text(identities, votes, top_ups),
                    MessageType::Warning,
                );
                handle.disable_auto_dismiss();
                let action = if identities > 0 && (votes > 0 || top_ups > 0) {
                    MIGRATION_UNREADABLE_ACK_ACTION_ID
                } else if identities > 0 {
                    MIGRATION_IDENTITIES_ACK_ACTION_ID
                } else {
                    MIGRATION_VOTES_ACK_ACTION_ID
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

    /// Drain pending banner-action clicks. Two kinds of action are registered:
    /// the migration Retry, which re-dispatches `FinishUnwire` after resetting the
    /// cold-start guard, and the three unreadable-row acknowledgements (votes,
    /// identities, or the combined banner naming both), each of which clears the
    /// durable warning records its banner named. All are returned for `AppState`
    /// to dispatch.
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
            } else if action_id == MIGRATION_VOTES_ACK_ACTION_ID {
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

#[cfg(test)]
mod tests {
    use super::*;
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
                votes: 0,
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
                votes: 3,
                top_ups: 0,
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

    /// The vote warning keeps its own acknowledgement — the identity work above
    /// must not have re-routed the sibling banner to the wrong task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_votes_banner_acknowledgement_routes_to_its_task() {
        let task = click_banner_action(
            MigrationState::SucceededWithUnreadableData {
                identities: 0,
                votes: 2,
                top_ups: 0,
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
            .prepare_gate
            .try_lock()
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
