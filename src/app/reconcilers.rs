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

use dash_sdk::dpp::dashcore::{Network, Txid};
use eframe::egui;

use crate::backend_task::error::TaskError;
use crate::backend_task::migration::{MigrationTask, migration_task_error};
use crate::backend_task::{BackendTask, platform_info};
use crate::context::AppContext;
use crate::context::connection_status::{
    OverallConnectionState, SPV_SYNC_PHASE_COUNT, spv_phase_step, spv_progress_token,
};
use crate::context::migration_status::MigrationState;
use crate::model::wallet::{TransactionConfirmation, WalletSeedHash};
use crate::ui::MessageType;
use crate::ui::components::wallet_unlock_popup::{
    MigrationWalletUnlockResult, WalletUnlockPopup, wallet_needs_unlock,
};
use crate::ui::components::{
    BannerHandle, MessageBanner, OptionBannerExt, OptionOverlayExt, OverlayConfig, OverlayHandle,
};

use super::{
    COLD_START_BACKEND_READY_TIMEOUT, COLD_START_STUCK_MESSAGE, MAX_PENDING_WATCHES,
    MIGRATION_IDENTITIES_ACK_ACTION_ID, MIGRATION_RETRY_ACTION_ID,
    MIGRATION_UNREADABLE_ACK_ACTION_ID, MIGRATION_VOTES_ACK_ACTION_ID, PENDING_CONFIRMED_MESSAGE,
    PENDING_POLL_INTERVAL, PENDING_STALE_MESSAGE, PendingStep, SPV_CONNECTING_DESCRIPTION,
    SPV_CONTINUE_BACKGROUND_ACTION, SPV_SYNCING_DESCRIPTION, SpvBlockStep,
    cold_start_backend_wait_timed_out, migration_failed_with_unreadable_identities_text,
    migration_running_text, migration_unreadable_data_text, pending_step,
    should_dispatch_cold_start, spv_block_step,
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

/// Reconciles the data-migration banner and drives the cold-start
/// `FinishUnwire` dispatch per network.
pub(super) struct MigrationReconciler {
    /// Handle to the current migration banner, if displayed.
    banner_handle: Option<BannerHandle>,
    /// Wallet-storage error cleared once migration is terminal and its run guard is free.
    storage_startup_error: TransientBanner,
    /// Last-seen migration state so reconciliation fires only on change.
    last_state: Option<MigrationState>,
    /// Networks whose cold-start `FinishUnwire` has been dispatched this process.
    dispatched: BTreeSet<Network>,
    /// Per network, when the readiness gate first observed an unwired backend.
    backend_wait_since: BTreeMap<Network, Instant>,
    /// Networks whose stuck-preparation timeout was already logged (dedupe).
    timeout_signaled: BTreeSet<Network>,
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
            dispatched: BTreeSet::new(),
            backend_wait_since: BTreeMap::new(),
            timeout_signaled: BTreeSet::new(),
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
    /// Each step / outcome surfaces a single i18n-ready sentence. Retryable
    /// failures get a "Retry now" action button.
    pub(super) fn update_banner(
        &mut self,
        ctx: &egui::Context,
        app_context: &Arc<AppContext>,
        frame_state: &MigrationState,
    ) {
        let state = frame_state.clone();
        let storage_guard_resolved = !matches!(
            &state,
            MigrationState::Idle
                | MigrationState::Running { .. }
                | MigrationState::AwaitingWalletPasswords { .. }
        ) && app_context.migration_run.try_lock().is_ok();
        self.storage_startup_error.clear_if(storage_guard_resolved);
        self.update_password_prompt(ctx, app_context, &state);
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
    /// The shared confirmation banner, held for the same reason the question
    /// it answers is: this feature exists because the user walked away, so
    /// the answer has to still be on screen when they come back. Raised
    /// without auto-dismiss — a default `Success` banner retires itself after
    /// five seconds, which would take the answer away moments after
    /// [`Self::sync_banners`] retired the question, leaving nothing at all.
    confirmed: Option<BannerHandle>,
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
            confirmed: None,
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
        // The confirmation names a payment on the network being left, so it
        // would be read against the wrong wallet's history if it stayed.
        self.confirmed.take_and_clear();
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
            // publishes is what we read, so retry on a later tick.
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
        let mut confirmed = false;
        for mut watch in self.watches.drain(..) {
            match pending_step(
                confirmation(&watch.txid),
                watch.since.elapsed(),
                watch.stale,
            ) {
                PendingStep::Confirmed => {
                    tracing::info!(txid = %watch.txid, "A payment with an unverified outcome is confirmed on the network");
                    confirmed = true;
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
        self.sync_banners(ctx);
        if confirmed {
            self.confirmed
                .raise_persistent(ctx, PENDING_CONFIRMED_MESSAGE, MessageType::Success);
        }
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
        reconciler.update_banner(&harness.ctx, &app_context, frame_state.as_ref());
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
        assert!(harness.query_by_label(PENDING_CONFIRMED_MESSAGE).is_some());
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
        adopt(
            &mut reconciler,
            &harness.ctx,
            Txid::from_byte_array([2u8; 32]),
        );

        reconciler.apply(&harness.ctx, |_| mined(1_234));
        harness.run();

        assert!(harness.query_by_label(PENDING_CONFIRMED_MESSAGE).is_some());
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
        assert!(harness.query_by_label(PENDING_CONFIRMED_MESSAGE).is_none());
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

        assert!(harness.query_by_label(PENDING_CONFIRMED_MESSAGE).is_some());
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
            .checked_sub(super::super::PENDING_STALE_AFTER)
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

        assert!(harness.query_by_label(PENDING_CONFIRMED_MESSAGE).is_some());
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
            .checked_sub(super::super::PENDING_STALE_AFTER)
            .expect("the test clock predates the stale threshold");

        reconciler.apply(&harness.ctx, |_| None);
        harness.run();
        assert!(harness.query_by_label(AMBIGUOUS).is_none());
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_some());
        assert_eq!(reconciler.watched(), vec![txid]);

        reconciler.apply(&harness.ctx, |_| mined(7));
        harness.run();
        assert!(harness.query_by_label(PENDING_STALE_MESSAGE).is_none());
        assert!(harness.query_by_label(PENDING_CONFIRMED_MESSAGE).is_some());
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
            .checked_sub(super::super::PENDING_STALE_AFTER)
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
            .checked_sub(super::super::PENDING_STALE_AFTER)
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
            .checked_sub(super::super::PENDING_STALE_AFTER)
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

        reconciler.update_banner(&harness.ctx, &app_context, &state);
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
        reconciler.update_banner(&harness.ctx, &app_context, state.as_ref());
        harness.run();
        assert!(harness.query_by_label(&message).is_some());

        let migration_guard = app_context
            .migration_run
            .try_lock()
            .expect("migration guard");
        app_context
            .migration_status()
            .set_state(MigrationState::Ready);
        let state = app_context.migration_status().state();
        reconciler.update_banner(&harness.ctx, &app_context, state.as_ref());
        harness.run();
        assert!(
            harness.query_by_label(&message).is_some(),
            "terminal state must not clear the startup error while storage remains locked",
        );

        drop(migration_guard);
        reconciler.update_banner(&harness.ctx, &app_context, state.as_ref());
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
