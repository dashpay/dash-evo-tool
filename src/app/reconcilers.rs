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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use dash_sdk::dpp::dashcore::Network;
use eframe::egui;

use crate::backend_task::migration::MigrationTask;
use crate::backend_task::{BackendTask, platform_info};
use crate::context::AppContext;
use crate::context::connection_status::{
    OverallConnectionState, SPV_SYNC_PHASE_COUNT, spv_phase_step, spv_progress_token,
};
use crate::context::migration_status::MigrationState;
use crate::ui::MessageType;
use crate::ui::components::{
    BannerHandle, MessageBanner, OptionOverlayExt, OverlayConfig, OverlayHandle,
};

use super::{
    COLD_START_BACKEND_READY_TIMEOUT, COLD_START_STUCK_MESSAGE, MIGRATION_IDENTITIES_ACK_ACTION_ID,
    MIGRATION_RETRY_ACTION_ID, MIGRATION_UNREADABLE_ACK_ACTION_ID, MIGRATION_VOTES_ACK_ACTION_ID,
    SPV_CONNECTING_DESCRIPTION, SPV_CONTINUE_BACKGROUND_ACTION, SPV_SYNCING_DESCRIPTION,
    SpvBlockStep, cold_start_backend_wait_timed_out,
    migration_failed_with_unreadable_identities_text, migration_running_text,
    migration_unreadable_identities_and_votes_text, migration_unreadable_identities_text,
    migration_unreadable_votes_text, should_dispatch_cold_start, spv_block_step,
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

/// Drives the blocking SPV-sync overlay (F-SPV-A). Owns the overlay handle and
/// the armed/dismissed episode flags so an ambient reconnect never hard-blocks
/// a working user.
pub(super) struct SpvBlockReconciler {
    /// The blocking overlay raised while an armed sync is connecting.
    overlay: Option<OverlayHandle>,
    /// Whether a user-initiated sync episode is armed for blocking.
    armed: bool,
    /// Whether the user chose "Continue in the background" for this episode.
    dismissed: bool,
}

impl SpvBlockReconciler {
    pub(super) fn new(armed: bool) -> Self {
        Self {
            overlay: None,
            armed,
            dismissed: false,
        }
    }

    /// Arm a fresh user-initiated episode (boot auto-start, Connect button,
    /// post-onboarding auto-start), re-arming the background escape.
    pub(super) fn arm(&mut self) {
        self.armed = true;
        self.dismissed = false;
    }

    /// Whether an episode is currently armed (test seam / observation).
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
        self.dismissed = false;
    }

    /// Drive the blocking SPV-sync overlay for one frame (see the field docs on
    /// [`AppState`](super::AppState) for the F-SPV-A / C1-C2 contract). Raises
    /// at most once per episode, then updates content in place.
    pub(super) fn update(&mut self, ctx: &egui::Context, app_context: &Arc<AppContext>) {
        let cs = app_context.connection_status();
        let state = cs.overall_state();
        match spv_block_step(self.armed, self.dismissed, state) {
            SpvBlockStep::Block => {
                // F-SPV-B: plain, jargon-free copy — the determinate granularity
                // is the "Step N of 5" counter, NOT raw phase names / heights.
                let progress = cs.spv_sync_progress();
                let step = progress.as_ref().and_then(spv_phase_step);
                // A-1 (Item B): the hidden liveness token tracks the advancing
                // height so a slow-but-advancing phase never trips the
                // no-progress watchdog. It is never rendered.
                let token = progress.as_ref().and_then(spv_progress_token);
                let description = if step.is_some() {
                    SPV_SYNCING_DESCRIPTION
                } else {
                    SPV_CONNECTING_DESCRIPTION
                };
                if self.overlay.is_none() {
                    // The escape is the single keyboard-reachable exit: the
                    // overlay focus-pins this button and lets Enter/Space
                    // activate it, so a keyboard-only / assistive-tech user is
                    // never stranded behind the UNBOUNDED SPV block.
                    let mut config = OverlayConfig::new()
                        .with_description(description)
                        .with_secondary_action(
                            "Continue in the background",
                            SPV_CONTINUE_BACKGROUND_ACTION,
                        )
                        .with_keyboard_escape(SPV_CONTINUE_BACKGROUND_ACTION);
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
                // ambient Connecting/Syncing never re-blocks (F-SPV-A). Re-arm
                // the escape for the next user-initiated sync.
                self.overlay.take_and_clear();
                self.armed = false;
                self.dismissed = false;
            }
            SpvBlockStep::Stand => {
                // User chose to continue in the background: stay lowered, but
                // keep the episode armed + dismissed so we don't re-raise (C2).
                self.overlay.take_and_clear();
            }
            SpvBlockStep::Idle => {
                // Not armed (ambient sync, or already disarmed): never block.
                self.overlay.take_and_clear();
            }
        }

        // Drain this overlay's own clicks: the "Continue in the background"
        // escape lowers the block for the rest of this episode.
        let actions = self
            .overlay
            .as_ref()
            .map(|handle| handle.take_actions())
            .unwrap_or_default();
        if actions
            .iter()
            .any(|id| id == SPV_CONTINUE_BACKGROUND_ACTION)
        {
            self.dismissed = true;
            self.overlay.take_and_clear();
        }
    }
}

/// Reconciles the connection-status banner with the overall connection state.
pub(super) struct ConnectionBanner {
    /// Previous state, to detect transitions. `None` forces re-evaluation.
    previous_state: Option<OverallConnectionState>,
    /// Handle to the current connection banner, if displayed.
    handle: Option<BannerHandle>,
}

impl ConnectionBanner {
    pub(super) fn new() -> Self {
        Self {
            previous_state: None,
            handle: None,
        }
    }

    /// Clear the banner and force re-evaluation next frame (network switch).
    pub(super) fn reset(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.clear();
        }
        self.previous_state = None;
    }

    /// Update the banner for the current connection state. `spv_overlaying`
    /// suppresses the redundant Connecting/Syncing copy while the SPV block is
    /// up. Returns a [`BackendTask`] to dispatch on the first `Synced`.
    pub(super) fn update(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
        spv_overlaying: bool,
    ) -> Option<BackendTask> {
        let connection_status = app_context.connection_status();
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

/// Reconciles the data-migration banner and drives the cold-start
/// `FinishUnwire` dispatch per network.
pub(super) struct MigrationReconciler {
    /// Handle to the current migration banner, if displayed.
    banner_handle: Option<BannerHandle>,
    /// Last-seen migration state so reconciliation fires only on change.
    last_state: Option<MigrationState>,
    /// Networks whose cold-start `FinishUnwire` has been dispatched this process.
    dispatched: BTreeSet<Network>,
    /// Per network, when the readiness gate first observed an unwired backend.
    backend_wait_since: BTreeMap<Network, Instant>,
    /// Networks whose stuck-preparation timeout was already logged (dedupe).
    timeout_signaled: BTreeSet<Network>,
}

impl MigrationReconciler {
    pub(super) fn new() -> Self {
        Self {
            banner_handle: None,
            last_state: None,
            dispatched: BTreeSet::new(),
            backend_wait_since: BTreeMap::new(),
            timeout_signaled: BTreeSet::new(),
        }
    }

    /// Clear the migration banner and force re-evaluation (network switch). The
    /// per-network dispatch guard is intentionally NOT reset — it is scoped per
    /// network so a return to a seen network never re-drains.
    pub(super) fn reset_for_switch(&mut self) {
        if let Some(handle) = self.banner_handle.take() {
            handle.clear();
        }
        self.last_state = None;
    }

    /// Dispatch the cold-start migration once per network, gated on the wallet
    /// backend being wired. Returns the `FinishUnwire` task when it fires;
    /// otherwise surfaces the stuck-preparation banner past the readiness
    /// timeout. See `finish_unwire` for the per-network scoping rationale.
    pub(super) fn dispatch_cold_start(
        &mut self,
        app_context: &Arc<AppContext>,
    ) -> Option<BackendTask> {
        let network = app_context.network;
        let already_dispatched = self.dispatched.contains(&network);
        // Readiness gate: after a network SWITCH the switched-to backend wires
        // a few frames later; dispatching before it is ready aborts the first
        // step with a transient error AND burns the per-network guard. Poll
        // readiness (a cheap ArcSwap load) and only dispatch once wired.
        let backend_ready = app_context.wallet_backend().is_ok();
        if should_dispatch_cold_start(already_dispatched, backend_ready) {
            // Backend wired: retire any stuck-preparation watchdog before
            // burning the guard and dispatching.
            self.clear_backend_wait(app_context);
            self.dispatched.insert(network);
            tracing::info!(
                target = "migration::cold_start",
                ?network,
                "Dispatching FinishUnwire migration at cold start",
            );
            return Some(BackendTask::MigrationTask(MigrationTask::FinishUnwire));
        }

        if already_dispatched {
            return None;
        }

        // Not dispatched because the wallet backend has not wired yet. Record
        // when the wait began and, once it exceeds the readiness timeout,
        // surface a visible, actionable banner. Recovery stays automatic: if the
        // backend wires later, the dispatch branch above clears the banner.
        let now = Instant::now();
        let waited = now.duration_since(*self.backend_wait_since.entry(network).or_insert(now));
        if cold_start_backend_wait_timed_out(Some(waited), COLD_START_BACKEND_READY_TIMEOUT) {
            let handle = MessageBanner::set_global(
                app_context.egui_ctx(),
                COLD_START_STUCK_MESSAGE,
                MessageType::Error,
            );
            handle.disable_auto_dismiss();
            // Log + attach the last wiring error once per network; the banner is
            // re-asserted every frame (idempotent) so it survives a switch.
            if self.timeout_signaled.insert(network) {
                if let Some(detail) = app_context.connection_status().spv_last_error() {
                    handle.with_details(detail);
                }
                tracing::warn!(
                    target = "migration::cold_start",
                    ?network,
                    waited_secs = waited.as_secs(),
                    "Wallet backend did not finish wiring within the readiness timeout; showing the wallet-preparation banner. Restart the app if this persists.",
                );
            }
        }
        None
    }

    /// Retire the stuck-preparation watchdog for the active network: drop the
    /// wait timer and, if the timeout banner was raised, remove it.
    fn clear_backend_wait(&mut self, app_context: &Arc<AppContext>) {
        let network = app_context.network;
        self.backend_wait_since.remove(&network);
        if self.timeout_signaled.remove(&network) {
            MessageBanner::clear_global_message(app_context.egui_ctx(), COLD_START_STUCK_MESSAGE);
        }
    }

    /// Update the migration banner to reflect the current [`MigrationState`].
    /// Each step / outcome surfaces a single i18n-ready sentence; `Failed` gets
    /// a "Retry now" action button.
    pub(super) fn update_banner(&mut self, ctx: &egui::Context, app_context: &Arc<AppContext>) {
        let state = (*app_context.migration_status().state()).clone();
        if self.last_state.as_ref() == Some(&state) {
            return;
        }
        self.last_state = Some(state.clone());

        // Clear the previous banner — text changes between steps must not stack.
        if let Some(handle) = self.banner_handle.take() {
            handle.clear();
        }

        match state {
            MigrationState::Idle => {}
            MigrationState::Running { step } => {
                let text = migration_running_text(step);
                let handle = MessageBanner::set_global(ctx, text, MessageType::Info);
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
            MigrationState::SucceededWithUnreadableVotes { count } => {
                // The wallets landed; only the corrupt vote rows did not. A
                // Warning (not Error): the drain is done and re-reading a corrupt
                // row cannot help, so there is no retry. Sticky, and re-raised on
                // every launch until the user clicks the acknowledge action — a
                // vote whose deadline still matters must not lose its only notice
                // because the user was away when the banner appeared.
                let handle = MessageBanner::set_global(
                    ctx,
                    migration_unreadable_votes_text(count),
                    MessageType::Warning,
                );
                handle.disable_auto_dismiss();
                handle.with_action("Got it", MIGRATION_VOTES_ACK_ACTION_ID);
                self.banner_handle = Some(handle);
            }
            MigrationState::SucceededWithUnreadableIdentities { count } => {
                // Same shape as the vote warning: the drain is done and the rows
                // are still in the previous version's storage, so no retry action
                // — but the keys those identities held are not loaded, so the user
                // must be told even if they stepped away. Sticky, and re-raised on
                // every launch until the "Got it" action retires the durable
                // record: a dismissal is not an acknowledgement.
                let handle = MessageBanner::set_global(
                    ctx,
                    migration_unreadable_identities_text(count),
                    MessageType::Warning,
                );
                handle.disable_auto_dismiss();
                handle.with_action("Got it", MIGRATION_IDENTITIES_ACK_ACTION_ID);
                self.banner_handle = Some(handle);
            }
            MigrationState::SucceededWithUnreadableIdentitiesAndVotes { identities, votes } => {
                // Both kinds of row were left behind. One Warning banner names both
                // remedies — no retry, since neither corrupt row decodes better on a
                // second pass. Sticky and acknowledgeable: because this single banner
                // names both problems, its one "Got it" retires both durable records
                // — re-raising either half afterwards would be a notice the user has
                // already read and acted on.
                let handle = MessageBanner::set_global(
                    ctx,
                    migration_unreadable_identities_and_votes_text(identities, votes),
                    MessageType::Warning,
                );
                handle.disable_auto_dismiss();
                handle.with_action("Got it", MIGRATION_UNREADABLE_ACK_ACTION_ID);
                self.banner_handle = Some(handle);
            }
            MigrationState::FailedWithUnreadableIdentities { count, error } => {
                // Both DET-owned passes broke on the same launch. One Error banner
                // (retryable, sticky) names both problems — the identities need
                // reloading AND the app-data update must be retried — so neither
                // silently hides the other.
                if error.is_backend_not_ready() {
                    // Transient app-data backend-not-ready: reset to Idle so the
                    // frame loop re-dispatches once ready, no failure flash.
                    self.dispatched.remove(&app_context.network);
                    app_context
                        .migration_status()
                        .set_state(MigrationState::Idle);
                    self.last_state = Some(MigrationState::Idle);
                    return;
                }
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
                if error.is_backend_not_ready() {
                    // Transient: the wallet backend had not finished wiring when
                    // this run fired. Drop the per-network dispatch guard so the
                    // frame loop re-dispatches once ready, and reset to Idle so
                    // no failure banner flashes — the retry is automatic.
                    self.dispatched.remove(&app_context.network);
                    app_context
                        .migration_status()
                        .set_state(MigrationState::Idle);
                    self.last_state = Some(MigrationState::Idle);
                    return;
                }
                let handle = MessageBanner::set_global(
                    ctx,
                    "Storage update could not complete. Your data is safe.",
                    MessageType::Error,
                );
                handle.disable_auto_dismiss();
                // The collapsed details panel + log line get the full typed
                // `MigrationError` chain rather than a lossy `to_string()`.
                handle.with_details(error.as_ref());
                handle.with_action("Retry now", MIGRATION_RETRY_ACTION_ID);
                self.banner_handle = Some(handle);
            }
        }
    }

    /// Dismiss the migration banner on Escape, unless the migration is still
    /// running (kept sticky so ongoing progress is not hidden).
    pub(super) fn handle_esc(&mut self, ctx: &egui::Context) {
        let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if !esc_pressed {
            return;
        }
        if matches!(
            self.last_state.as_ref(),
            Some(MigrationState::Running { .. })
        ) {
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
                // Reset the reconciler so the new run's Running banner overwrites
                // the stale Failed one, and drop the per-network dispatch guard
                // so a future `dispatch_cold_start` for the same network re-fires.
                self.last_state = None;
                self.dispatched.remove(&network);
                task = Some(BackendTask::MigrationTask(MigrationTask::FinishUnwire));
            } else if action_id == MIGRATION_VOTES_ACK_ACTION_ID {
                tracing::info!(
                    target = "migration::cold_start",
                    ?network,
                    "User acknowledged the unreadable-vote warning",
                );
                task = Some(BackendTask::MigrationTask(
                    MigrationTask::AcknowledgeUnreadableVotes,
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
                    "User acknowledged the combined unreadable-identity and unreadable-vote warning",
                );
                task = Some(BackendTask::MigrationTask(
                    MigrationTask::AcknowledgeUnreadableIdentitiesAndVotes,
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

        reconciler.update_banner(&harness.ctx, &app_context);
        harness.run();
        harness.get_by_label(label).click();
        harness.run();

        reconciler.drain_actions(&harness.ctx, app_context.network)
    }

    /// The unreadable-identity warning is acknowledgeable. It used to render as a
    /// sticky banner with NO action button at all: the user was told their signing
    /// keys had not come across and given no way to say "I understand", so the
    /// warning returned on every launch with no gesture that could retire it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_identities_banner_acknowledgement_routes_to_its_task() {
        let task = click_banner_action(
            MigrationState::SucceededWithUnreadableIdentities { count: 2 },
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
            MigrationState::SucceededWithUnreadableIdentitiesAndVotes {
                identities: 2,
                votes: 3,
            },
            "Got it",
        );
        assert_eq!(
            task,
            Some(BackendTask::MigrationTask(
                MigrationTask::AcknowledgeUnreadableIdentitiesAndVotes
            )),
            "one banner naming both problems must retire both warnings on one click",
        );
    }

    /// The vote warning keeps its own acknowledgement — the identity work above
    /// must not have re-routed the sibling banner to the wrong task.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_votes_banner_acknowledgement_routes_to_its_task() {
        let task = click_banner_action(
            MigrationState::SucceededWithUnreadableVotes { count: 2 },
            "Got it",
        );
        assert_eq!(
            task,
            Some(BackendTask::MigrationTask(
                MigrationTask::AcknowledgeUnreadableVotes
            )),
        );
    }
}
