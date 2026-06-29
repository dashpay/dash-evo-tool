//! Full-window blocking progress overlay.
//!
//! A sibling to [`MessageBanner`](super::message_banner) for operations that are
//! unsafe or meaningless to interact *around* — broadcasting a state transition,
//! signing, key import, a multi-step registration, a migration. It draws a
//! dimming plane over the whole window, blocks all interaction beneath it, and
//! shows an indeterminate [`egui::Spinner`] (no ETA), an optional discrete step
//! counter, an optional description, and optional generic action buttons.
//!
//! Like the banner, it is a **visual + input** block only — it never waits in
//! the frame loop. The owning operation runs on a tokio `BackendTask`; the
//! overlay is raised at dispatch and lowered when the `TaskResult` is polled.
//!
//! ## Buttons are a generic facility (no built-in Cancel)
//!
//! The overlay has **no** Cancel concept. A caller attaches a generic button
//! with [`OverlayConfig::with_action`] / [`OverlayHandle::with_action`] (or the
//! `*_secondary_action` variants), picking its own opaque action id and label.
//! Clicking the button enqueues that action id, keyed by the owning entry; the
//! overlay does **not** auto-lower. The owning screen drains **its own** ids via
//! [`OverlayHandle::take_actions`] (FIFO) at the top of its `ui()` and decides
//! what to do — including running its own cancellation logic if it labelled a
//! button "Cancel". The app loop only sweeps orphaned ids via
//! [`ProgressOverlay::sweep_orphan_actions`].
//!
//! ## Two render paths (mirrors `MessageBanner`)
//!
//! Like `MessageBanner`, this type has both an instance [`Component`] path and a
//! global path, sharing one layout helper ([`render_card`]):
//!
//! - **Global** — state lives in egui `ctx.data`; [`ProgressOverlay::set_global`]
//!   raises it, [`ProgressOverlay::render_global`] paints the full-window dim +
//!   input sink + centered card once per frame from `AppState::update`. This is
//!   the app-level blocking path the application depends on.
//! - **Instance** — `ProgressOverlay { state }` configured via builder methods,
//!   rendered inline by [`Component::show`]. It paints only the card (no dim/sink)
//!   and surfaces a clicked button's action id through [`ProgressOverlayResponse`].
//!
//! ## Concurrency (global path)
//!
//! Global state is a **stack** of active requests. The overlay is visible while
//! the stack is non-empty; the topmost (last-pushed) entry is rendered; each
//! handle dismisses only its own entry; the overlay clears when the stack empties.
//! A human cannot launch a second blocker, so concurrency only arises from
//! programmatic tasks — the stack guarantees the UI never unblocks while an
//! operation is still running.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use egui::InnerResponse;
use tracing::{debug, warn};

use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{ComponentStyles, DashColors, Shadow, Shape, Spacing};

const OVERLAY_STATE_ID: &str = "__global_progress_overlay";
const OVERLAY_ACTIONS_ID: &str = "__global_progress_overlay_actions";
const OVERLAY_DIM_SINK_ID: &str = "__global_progress_overlay_sink";
const OVERLAY_CARD_ID: &str = "__global_progress_overlay_card";

/// After this long on the topmost request the renderer auto-reveals the honest
/// elapsed readout and a reassurance line. Visual only — never auto-aborts.
const STUCK_OVERLAY_THRESHOLD: Duration = Duration::from_secs(30);

/// After this long *without progress* on the topmost request, escalate the
/// reassurance copy and fire the one-shot developer watchdog (A-1). A leaked
/// handle (C1) or an un-bounded op (C2) is the usual cause — both are bugs.
const STUCK_OVERLAY_WATCHDOG_THRESHOLD: Duration = Duration::from_secs(120);

/// Diameter of the indeterminate spinner inside the card.
const SPINNER_SIZE: f32 = 32.0;
/// Card minimum width so short content still reads as a deliberate dialog.
const CARD_MIN_WIDTH: f32 = 240.0;
/// Card maximum width so long descriptions wrap instead of stretching.
const CARD_MAX_WIDTH: f32 = 420.0;
/// Description scrolls inside the card past this height (FR-6: never off-screen).
const DESCRIPTION_MAX_HEIGHT: f32 = 160.0;

/// Reassurance line revealed once the soft 30 s stuck threshold passes.
const STUCK_REASSURANCE: &str = "This is taking longer than usual.";

/// Escalated reassurance shown once the 120 s no-progress watchdog trips,
/// replacing (not stacking with) [`STUCK_REASSURANCE`].
const STUCK_WATCHDOG_REASSURANCE: &str = "This is taking much longer than expected. The operation is still running — please keep the app open.";

/// Monotonic counter for generating unique overlay keys.
static OVERLAY_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_overlay_key() -> u64 {
    // Relaxed is sufficient: we only need uniqueness, not ordering. The counter
    // runs in a single-threaded UI context.
    OVERLAY_KEY_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Visual tier of an overlay button (F-3/F-4/F-7). Styling and placement only —
/// both tiers are generic and carry no built-in semantics (there is no Cancel).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ButtonStyle {
    /// Accent fill; hugs the right edge — the affirmative / continue action.
    Primary,
    /// Muted fill; sits to the left of the primary — e.g. a caller's "cancel".
    Secondary,
}

/// One generic action button on the overlay card. The caller owns both the
/// `label` (an i18n unit, user-visible and logged) and the opaque `action_id`
/// enqueued on click. `style` controls appearance and placement only.
#[derive(Clone)]
struct OverlayButton {
    label: String,
    action_id: String,
    style: ButtonStyle,
}

impl OverlayButton {
    fn new(id: impl fmt::Display, label: impl fmt::Display, style: ButtonStyle) -> Self {
        Self {
            label: label.to_string(),
            action_id: id.to_string(),
            style,
        }
    }
}

/// Snapshot of the content fields logged for change-detection: the description
/// and the step pair. Compared to log a content update exactly once (NFR-5).
type LoggedContent = (Option<String>, Option<(u32, u32)>);

/// One active blocking request on the overlay stack (and the per-instance state
/// for the [`Component`] path).
#[derive(Clone)]
struct OverlayState {
    key: u64,
    description: Option<String>,
    /// Raw, unvalidated; the renderer gates on [`step_is_renderable`].
    step: Option<(u32, u32)>,
    buttons: Vec<OverlayButton>,
    /// Explicit opt-in; also forced once `created_at` passes the threshold.
    show_elapsed: bool,
    created_at: Instant,
    /// Set once the overlay has been logged as shown (NFR-5).
    logged: bool,
    /// Last content logged, so a description/step update logs exactly once.
    logged_content: Option<LoggedContent>,
    /// Hidden, monotonic liveness token (A-1). Never rendered. An owner that drives
    /// a long phase whose shown `(description, step)` is constant (e.g. SPV headers)
    /// advances this from the underlying progress (a climbing height) so the
    /// watchdog can tell a slow-but-advancing phase from a genuine stall.
    progress_token: Option<u64>,
    /// The `progress_token` value at the last watchdog reset, for change detection.
    last_progress_token: Option<u64>,
    /// Last time real progress was seen — either the shown `(description, step)`
    /// changed OR the hidden `progress_token` advanced. The no-progress watchdog
    /// (A-1) measures from here, so a legitimately advancing flow (multi-step or a
    /// single slow-but-advancing phase) never trips it while a genuinely wedged
    /// operation does.
    last_progress_at: Instant,
    /// Set once the no-progress watchdog has fired its one-shot dev-error (A-1),
    /// so the error logs exactly once, never per frame (NFR-5).
    watchdog_logged: bool,
    /// Set once focus has been placed on the first button (focus trap).
    focus_requested: bool,
    /// Opt-in action id designated as the single keyboard-reachable escape (QA-002
    /// refinement). When set, `claim_input` activates it at frame start: a press of
    /// Enter/Space enqueues this action directly (the same queue a click feeds) and
    /// is stripped like every other key, so the escape needs no focus and the key
    /// never reaches a widget beneath. `None` by default — a block is fully
    /// keyboard-blocked unless it opts in.
    keyboard_escape_action: Option<String>,
}

impl OverlayState {
    fn new(key: u64, description: Option<String>, config: &OverlayConfig) -> Self {
        let now = Instant::now();
        Self {
            key,
            description,
            step: config.step,
            buttons: config.buttons.clone(),
            show_elapsed: config.show_elapsed,
            created_at: now,
            logged: false,
            logged_content: None,
            progress_token: config.progress_token,
            last_progress_token: config.progress_token,
            last_progress_at: now,
            watchdog_logged: false,
            focus_requested: false,
            keyboard_escape_action: config.keyboard_escape_action.clone(),
        }
    }
}

/// Builder/config for [`ProgressOverlay::set_global`]. `OverlayConfig::default()`
/// is a spinner-only block: no counter, no buttons, elapsed off.
#[derive(Clone, Default)]
pub struct OverlayConfig {
    description: Option<String>,
    step: Option<(u32, u32)>,
    show_elapsed: bool,
    buttons: Vec<OverlayButton>,
    /// Hidden liveness token (A-1). Never rendered; see [`with_progress_token`].
    ///
    /// [`with_progress_token`]: Self::with_progress_token
    progress_token: Option<u64>,
    /// Opt-in keyboard escape (QA-002 refinement); see [`with_keyboard_escape`].
    ///
    /// [`with_keyboard_escape`]: Self::with_keyboard_escape
    keyboard_escape_action: Option<String>,
}

impl OverlayConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the description. The `description` argument of `set_global` wins when
    /// this is unset, so most callers pass the text there instead.
    pub fn with_description(mut self, text: impl fmt::Display) -> Self {
        let text = text.to_string();
        self.description = (!text.is_empty()).then_some(text);
        self
    }

    pub fn with_step(mut self, current: u32, total: u32) -> Self {
        self.step = Some((current, total));
        self
    }

    /// Show the honest count-up elapsed readout from the start.
    pub fn with_elapsed(mut self) -> Self {
        self.show_elapsed = true;
        self
    }

    /// Seed the **hidden** liveness token (A-1). NOT rendered to the user — it only
    /// feeds the no-progress watchdog: a change in the token between frames counts
    /// as progress and resets the watchdog clock, so a slow-but-still-advancing
    /// operation (e.g. SPV headers on a slow link, where the shown "Step N of 5"
    /// stays constant for minutes) never trips the false-stall escalation. Most
    /// callers update it each frame via [`OverlayHandle::set_progress_token`].
    pub fn with_progress_token(mut self, token: u64) -> Self {
        self.progress_token = Some(token);
        self
    }

    /// Add a **primary** action button (accent fill, hugs the right edge). Mirrors
    /// [`MessageBanner::with_action`](super::message_banner::MessageBanner::with_action):
    /// the displayed `label` comes first, the opaque `action_id` enqueued on click
    /// second. Clicking does not lower the overlay — the owning screen drains the
    /// id and decides what to do.
    ///
    /// Buttons render right-to-left in the order added: primaries hug the right
    /// edge, secondaries sit to their left. SEC-006: `label` and `action_id` are
    /// user-visible and logged — never pass secrets or PII.
    pub fn with_action(mut self, label: impl fmt::Display, action_id: impl fmt::Display) -> Self {
        self.buttons
            .push(OverlayButton::new(action_id, label, ButtonStyle::Primary));
        self
    }

    /// Add a **secondary** action button (muted fill, sits left of the primary).
    /// Same generic semantics as [`with_action`](Self::with_action) — only the
    /// styling and placement differ; there is no built-in Cancel. SEC-006: `label`
    /// and `action_id` are user-visible and logged — never pass secrets or PII.
    pub fn with_secondary_action(
        mut self,
        label: impl fmt::Display,
        action_id: impl fmt::Display,
    ) -> Self {
        self.buttons
            .push(OverlayButton::new(action_id, label, ButtonStyle::Secondary));
        self
    }

    /// Designate one already-added action as the single **keyboard-reachable
    /// escape** (QA-002 refinement). Pass the same opaque `action_id` you gave to
    /// [`with_action`](Self::with_action) / [`with_secondary_action`](Self::with_secondary_action).
    ///
    /// A hard block is otherwise never keyboard-activatable: `claim_input` strips
    /// every navigation/confirm key every frame so a focused button cannot be
    /// triggered by keyboard. This opt-in carves out exactly one exception — the
    /// designated button stays activatable with Enter or Space — for blocks that are
    /// **unbounded** and would otherwise strand a keyboard-only / assistive-tech user
    /// (the reference adopter is the SPV-sync block, whose sync can wait
    /// indefinitely for peers). The overlay focus-pins the escape button so the
    /// passthrough can never reach a widget beneath; every OTHER hard block, and
    /// everything beneath, stays fully keyboard-blocked. Designating an action id
    /// that no button carries is a no-op.
    pub fn with_keyboard_escape(mut self, action_id: impl fmt::Display) -> Self {
        self.keyboard_escape_action = Some(action_id.to_string());
        self
    }
}

/// Lifecycle handle for a raised overlay, returned by
/// [`ProgressOverlay::set_global`]. Identifies its entry by an internal key, so
/// content can be updated without losing the reference. Methods are no-ops
/// returning `None` once the entry is gone.
///
/// INTENTIONAL(SEC-005): `OverlayHandle` is `Send + Sync` only because it holds
/// an `egui::Context` (itself `Send + Sync` via internal locking). That does NOT
/// make handle operations thread-safe to interleave: every method reads-modifies-
/// writes the global `ctx.data` overlay slot non-atomically, so the real
/// invariant is that all handle operations run on the single egui UI/update
/// thread (where the overlay is shown, mutated, and cleared). The `Send + Sync`
/// derivation is incidental to `egui::Context`'s bounds, not a claim of
/// cross-thread correctness.
#[derive(Clone)]
pub struct OverlayHandle {
    ctx: egui::Context,
    key: u64,
}

impl OverlayHandle {
    /// Whether this handle's entry is still on the stack.
    pub fn is_active(&self) -> bool {
        get_overlay_state(&self.ctx)
            .iter()
            .any(|s| s.key == self.key)
    }

    /// How long ago this entry was raised, or `None` if it is gone.
    pub fn elapsed(&self) -> Option<Duration> {
        get_overlay_state(&self.ctx)
            .iter()
            .find(|s| s.key == self.key)
            .map(|s| s.created_at.elapsed())
    }

    /// Update the description in place. Returns `None` if the entry is gone.
    pub fn set_description(&self, text: impl fmt::Display) -> Option<&Self> {
        self.mutate(|s| {
            let text = text.to_string();
            s.description = (!text.is_empty()).then_some(text);
        })
    }

    /// Update the step counter in place. Returns `None` if the entry is gone.
    pub fn set_step(&self, current: u32, total: u32) -> Option<&Self> {
        self.mutate(|s| s.step = Some((current, total)))
    }

    /// Remove the step counter. Returns `None` if the entry is gone.
    pub fn clear_step(&self) -> Option<&Self> {
        self.mutate(|s| s.step = None)
    }

    /// Attach a **primary** action button (accent fill, right edge). Mirrors
    /// [`MessageBanner::with_action`](super::message_banner::MessageBanner::with_action):
    /// `label` shown verbatim first, opaque `action_id` enqueued on click second.
    /// Buttons render right-to-left in the order added. SEC-006: `label`/`action_id`
    /// are user-visible and logged — never pass secrets or PII. Returns `None` if
    /// the entry is gone.
    pub fn with_action(
        &self,
        label: impl fmt::Display,
        action_id: impl fmt::Display,
    ) -> Option<&Self> {
        let button = OverlayButton::new(action_id, label, ButtonStyle::Primary);
        self.mutate(|s| s.buttons.push(button))
    }

    /// Attach a **secondary** action button (muted fill, left of the primary).
    /// Same generic semantics as [`with_action`](Self::with_action). SEC-006:
    /// `label`/`action_id` are user-visible and logged. Returns `None` if the entry
    /// is gone.
    pub fn with_secondary_action(
        &self,
        label: impl fmt::Display,
        action_id: impl fmt::Display,
    ) -> Option<&Self> {
        let button = OverlayButton::new(action_id, label, ButtonStyle::Secondary);
        self.mutate(|s| s.buttons.push(button))
    }

    /// Update the **hidden** liveness token in place (A-1). NOT rendered — it only
    /// feeds the no-progress watchdog so an advancing underlying operation (e.g. an
    /// SPV phase whose height climbs while the shown "Step N of 5" stays constant)
    /// resets the watchdog clock. Returns `None` if the entry is gone.
    pub fn set_progress_token(&self, token: u64) -> Option<&Self> {
        self.mutate(|s| s.progress_token = Some(token))
    }

    /// Designate one already-attached action as the single keyboard-reachable
    /// escape, mirroring [`OverlayConfig::with_keyboard_escape`]. Pass the same
    /// opaque `action_id` you gave to a `with_action` / `with_secondary_action`
    /// call. Returns `None` if the entry is gone.
    pub fn with_keyboard_escape(&self, action_id: impl fmt::Display) -> Option<&Self> {
        let action_id = action_id.to_string();
        self.mutate(|s| s.keyboard_escape_action = Some(action_id))
    }

    /// Drain (FIFO) and remove the action ids enqueued by **this handle's**
    /// button clicks, leaving other overlay entries' actions untouched (A-3).
    ///
    /// The owning screen calls this at the top of its own `ui()` each frame and
    /// matches its own colon-namespaced ids (e.g. `shielded:build:cancel`),
    /// running whatever logic it owns — including its own cancellation. Returns
    /// an empty `Vec` when this handle has no pending clicks (or is already gone).
    pub fn take_actions(&self) -> Vec<String> {
        let mut queue = get_overlay_actions(&self.ctx);
        let mut mine = Vec::new();
        queue.retain(|a| {
            if a.key == self.key {
                mine.push(a.action_id.clone());
                false
            } else {
                true
            }
        });
        if !mine.is_empty() {
            set_overlay_actions(&self.ctx, queue);
        }
        mine
    }

    /// Test clock seam (RQ-2): shift this entry's `created_at` **and**
    /// `last_progress_at` into the past by `by`, so a kittest can render past the
    /// 30 s soft-reveal and 120 s no-progress watchdog thresholds without waiting.
    /// Returns `None` if the entry is gone. Compiled only under the `testing`
    /// feature — never part of the production surface.
    #[cfg(feature = "testing")]
    pub fn backdate(&self, by: Duration) -> Option<&Self> {
        self.mutate(|s| {
            if let Some(t) = s.created_at.checked_sub(by) {
                s.created_at = t;
            }
            if let Some(t) = s.last_progress_at.checked_sub(by) {
                s.last_progress_at = t;
            }
        })
    }

    /// Dismiss only this handle's entry, and purge any of its still-pending action
    /// ids so a normal dismiss leaves nothing for the orphan-sweeper (A-3). The
    /// overlay lowers when the stack empties.
    pub fn clear(self) {
        let mut stack = get_overlay_state(&self.ctx);
        let before = stack.len();
        stack.retain(|s| s.key != self.key);
        if stack.len() != before {
            debug!(key = self.key, "Blocking progress overlay dismissed");
        }
        set_overlay_state(&self.ctx, stack);

        let mut queue = get_overlay_actions(&self.ctx);
        let kept = queue.len();
        queue.retain(|a| a.key != self.key);
        if queue.len() != kept {
            set_overlay_actions(&self.ctx, queue);
        }
    }

    /// Find this handle's entry, apply `f`, and write the stack back. The next
    /// `log_overlay_state` detects the content change (it compares
    /// `(description, step)`), logs the update once, and bumps `last_progress_at`
    /// for the no-progress watchdog — so this method intentionally does not touch
    /// `logged_content` itself.
    fn mutate(&self, f: impl FnOnce(&mut OverlayState)) -> Option<&Self> {
        let mut stack = get_overlay_state(&self.ctx);
        let entry = stack.iter_mut().find(|s| s.key == self.key)?;
        f(entry);
        set_overlay_state(&self.ctx, stack);
        Some(self)
    }
}

/// Response returned by [`ProgressOverlay::show`] (the [`Component`] path).
///
/// The only thing a user can change about an overlay is clicking one of its
/// generic buttons, so the domain value is the clicked button's **action id**.
/// `changed_value` is `Some(action_id)` for the single frame a button is clicked
/// and `None` otherwise; the overlay is never in an "invalid" state.
#[derive(Clone)]
pub struct ProgressOverlayResponse {
    action: Option<String>,
    changed: bool,
}

impl ComponentResponse for ProgressOverlayResponse {
    type DomainType = String;

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.action
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// The blocking progress overlay.
///
/// Two paths share the [`render_card`] layout helper (mirrors `MessageBanner`):
/// the global `ctx.data` path ([`set_global`](Self::set_global) /
/// [`render_global`](Self::render_global)), driven once per frame from
/// `AppState::update`, and the instance [`Component`] path configured by the
/// builder methods and rendered by [`Component::show`].
pub struct ProgressOverlay {
    state: Option<OverlayState>,
    /// The action id of the most recently clicked button on this instance,
    /// surfaced by [`Component::current_value`]. `None` until a click occurs.
    last_action: Option<String>,
}

impl Default for ProgressOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressOverlay {
    // ── Instance (Component) path ───────────────────────────────────────────

    /// Create a spinner-only instance overlay. Configure it with the builder
    /// methods, then render it inline via [`Component::show`].
    pub fn new() -> Self {
        let key = next_overlay_key();
        Self {
            state: Some(OverlayState::new(key, None, &OverlayConfig::default())),
            last_action: None,
        }
    }

    /// Set the description shown beneath the spinner. An empty string clears it.
    pub fn with_description(mut self, text: impl fmt::Display) -> Self {
        if let Some(state) = &mut self.state {
            let text = text.to_string();
            state.description = (!text.is_empty()).then_some(text);
        }
        self
    }

    /// Show a discrete step counter ("Step {current} of {total}").
    pub fn with_step(mut self, current: u32, total: u32) -> Self {
        if let Some(state) = &mut self.state {
            state.step = Some((current, total));
        }
        self
    }

    /// Show the honest count-up elapsed readout from the start.
    pub fn with_elapsed(mut self) -> Self {
        if let Some(state) = &mut self.state {
            state.show_elapsed = true;
        }
        self
    }

    /// Add a **primary** action button. Mirrors
    /// [`MessageBanner::with_action`](super::message_banner::MessageBanner::with_action):
    /// the `label` shown on the button comes first, the opaque `action_id` surfaced
    /// through [`ProgressOverlayResponse`] on click second.
    pub fn with_action(mut self, label: impl fmt::Display, action_id: impl fmt::Display) -> Self {
        if let Some(state) = &mut self.state {
            state
                .buttons
                .push(OverlayButton::new(action_id, label, ButtonStyle::Primary));
        }
        self
    }

    /// Add a **secondary** action button (left of the primary). Same generic
    /// semantics as [`with_action`](Self::with_action).
    pub fn with_secondary_action(
        mut self,
        label: impl fmt::Display,
        action_id: impl fmt::Display,
    ) -> Self {
        if let Some(state) = &mut self.state {
            state
                .buttons
                .push(OverlayButton::new(action_id, label, ButtonStyle::Secondary));
        }
        self
    }

    /// Clear this instance so [`Component::show`] renders nothing and returns the
    /// empty response (QA-007: makes the `state == None` path reachable via the
    /// public API). Idempotent.
    pub fn clear(&mut self) {
        self.state = None;
    }

    // ── Global (ctx.data) path ──────────────────────────────────────────────

    /// Raise the overlay and return its handle. Non-blocking: only writes
    /// `ctx.data`. The `description` argument is used unless `config` already
    /// carries one.
    ///
    /// **Lifecycle (SEC-001):** a button-less app-level block has no automatic
    /// teardown — the no-progress watchdog only *logs*, it never lowers the block.
    /// A button-less block MUST therefore either be driven by a frame-driven
    /// reconcile owner that lowers it when the work ends (the reference pattern is
    /// the SPV adopter, `AppState::update_spv_overlay`), or carry an escape
    /// button; a leaked or forgotten handle strands the UI with no way out.
    ///
    /// SEC-006: the `description` (and any button `label`/`id`) is user-visible
    /// and written to logs on show — never pass secrets, passphrases, or PII.
    pub fn set_global(
        ctx: &egui::Context,
        description: impl fmt::Display,
        config: OverlayConfig,
    ) -> OverlayHandle {
        let description = config.description.clone().or_else(|| {
            let text = description.to_string();
            (!text.is_empty()).then_some(text)
        });
        let key = next_overlay_key();
        let mut stack = get_overlay_state(ctx);
        if !stack.is_empty() {
            // Logged here (once per show), never per frame — a blocking overlay
            // cannot be stacked by a human, so this signals a programmatic smell.
            warn!(
                key,
                depth = stack.len(),
                "A blocking overlay was requested while another is active"
            );
        }
        stack.push(OverlayState::new(key, description, &config));
        set_overlay_state(ctx, stack);
        OverlayHandle {
            ctx: ctx.clone(),
            key,
        }
    }

    /// Convenience: a spinner-only block with no text, counter, or buttons.
    ///
    /// As a button-less block it has no escape, so the SEC-001 lifecycle rule from
    /// [`set_global`](Self::set_global) applies in full: drive it from a
    /// frame-driven reconcile owner (e.g. `AppState::update_spv_overlay`) that
    /// lowers it when the work ends — a leaked handle has no automatic teardown.
    pub fn set_global_spinner_only(ctx: &egui::Context) -> OverlayHandle {
        Self::set_global(ctx, "", OverlayConfig::default())
    }

    /// Whether any overlay is active. Cheap one-slot read (NFR-6).
    pub fn has_global(ctx: &egui::Context) -> bool {
        !get_overlay_state(ctx).is_empty()
    }

    /// Orphan-sweeper (A-3): drain and return only action ids whose owning
    /// overlay entry is **no longer on the stack** — i.e. the owner cleared or
    /// dropped its handle without draining. Live owners' actions are left
    /// untouched, so this can never race or pre-empt a screen that owns an active
    /// overlay, regardless of call order. The app loop calls this and logs each
    /// truly-orphaned id; screens drain their own via [`OverlayHandle::take_actions`].
    pub fn sweep_orphan_actions(ctx: &egui::Context) -> Vec<String> {
        let live: std::collections::HashSet<u64> =
            get_overlay_state(ctx).iter().map(|s| s.key).collect();
        let mut queue = get_overlay_actions(ctx);
        let mut orphans = Vec::new();
        queue.retain(|a| {
            if live.contains(&a.key) {
                true
            } else {
                orphans.push(a.action_id.clone());
                false
            }
        });
        if !orphans.is_empty() {
            set_overlay_actions(ctx, queue);
        }
        orphans
    }

    /// Clear every entry — used on network switch alongside the banner reset.
    ///
    /// SEC-007: also clears the pending action queue, so a click queued just
    /// before a network switch cannot survive into the new context and be
    /// mis-dispatched there.
    pub fn clear_all_global(ctx: &egui::Context) {
        set_overlay_state(ctx, Vec::new());
        set_overlay_actions(ctx, Vec::new());
    }

    /// Claim all keyboard and text input for the active block, at frame start.
    ///
    /// Must be called near the top of `AppState::update` — **before** the panels
    /// and the visible screen run — and the caller MUST skip it while a secret
    /// prompt is active above the overlay (that modal needs the keyboard).
    /// Early-outs when no overlay is active.
    ///
    /// Why a separate frame-start pass: `render_global`'s own key filter runs at
    /// the *end* of the frame, one frame too late for a button-less block raised
    /// over an already-focused field — the field beneath has already consumed the
    /// keystroke. `claim_input` closes that leak (QA-001) by, while a block is up:
    /// - releasing text-edit focus from any field beneath (so it stops drawing a
    ///   caret and consuming text — affects only text widgets, never an overlay
    ///   button), and
    /// - stripping `Event::Text`, the clipboard events (Copy/Cut/Paste), and the
    ///   navigation/confirm/edit keys (Tab, Enter, Escape, Space, arrows,
    ///   Backspace, Delete, Home, End, PageUp, PageDown) from `i.events` so
    ///   nothing beneath observes them.
    ///
    /// A hard block is never keyboard-dismissable or keyboard-activatable, with one
    /// opt-in exception: a block that designates a single keyboard escape via
    /// [`OverlayConfig::with_keyboard_escape`]. For such a block, a frame-start press
    /// of **Enter or Space** enqueues the designated action directly — the same queue
    /// a click feeds — and the key is then stripped along with every other one. The
    /// activation happens here, before the beneath `ui()` runs, so it needs no focus
    /// (SEC-001) and the key never survives to a widget beneath (SEC-002). Every
    /// other key, and every non-opted block, stays fully blocked.
    pub fn claim_input(ctx: &egui::Context) {
        let stack = get_overlay_state(ctx);
        let Some(top) = stack.last() else {
            return;
        };
        // Release beneath focus ONLY for a button-less block — it has no widget of
        // its own to hold focus, so a focused field beneath would keep its caret.
        // A buttoned block keeps its button focused (`render_buttons` manages the
        // focus + lock), so do NOT clear focus here — `stop_text_input` clears the
        // *currently focused* widget regardless of type, which would steal the
        // button's focus every frame.
        if top.buttons.is_empty() {
            ctx.memory_mut(|m| m.stop_text_input());
        }
        // A designated keyboard escape is activated HERE, at frame start: a press of
        // Enter/Space enqueues its action directly (the same queue a click feeds) and
        // is then stripped like every other key. Doing it before the beneath `ui()`
        // runs means the activation needs no focus (SEC-001) and the key never
        // survives to a focus-independent handler beneath (SEC-002). A non-opted block
        // enqueues nothing and strips Enter/Space exactly the same.
        let escape_action = top.keyboard_escape_action.clone();
        let key = top.key;
        let mut activate_escape = false;
        ctx.input_mut(|i| {
            i.events.retain(|e| {
                if matches!(
                    e,
                    egui::Event::Text(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Paste(_)
                ) {
                    return false;
                }
                if let egui::Event::Key {
                    key: egui::Key::Enter | egui::Key::Space,
                    pressed: true,
                    repeat,
                    ..
                } = e
                {
                    // Enqueue once per real press (ignore key-repeat); always strip.
                    if escape_action.is_some() && !*repeat {
                        activate_escape = true;
                    }
                    return false;
                }
                !matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Tab
                            | egui::Key::Escape
                            | egui::Key::ArrowUp
                            | egui::Key::ArrowDown
                            | egui::Key::ArrowLeft
                            | egui::Key::ArrowRight
                            | egui::Key::Backspace
                            | egui::Key::Delete
                            | egui::Key::Home
                            | egui::Key::End
                            | egui::Key::PageUp
                            | egui::Key::PageDown,
                        pressed: true,
                        ..
                    }
                )
            });
        });
        // Enqueue after releasing the input lock — `push_overlay_action` takes the
        // `ctx.data` lock, which must not nest inside `ctx.input_mut`.
        if activate_escape && let Some(action_id) = escape_action {
            push_overlay_action(ctx, key, &action_id);
        }
        // TODO(SEC-002-pointer): claim pointer press/click/drag at frame start
        // (analogue of the keyboard QA-001 frame-start claim) to close the
        // one-frame click-through on the raising frame.
    }

    /// Render the topmost entry. Call once per frame from `AppState::update`,
    /// after the panels and before the secret prompt. Early-outs to a single
    /// `ctx.data` read when no overlay is active (NFR-6).
    ///
    /// `secret_prompt_active` mirrors the [`claim_input`](Self::claim_input)
    /// secret-prompt gate: when `true` the block suppresses its own focus management
    /// so the passphrase modal rendered above it keeps the keyboard (SEC-001).
    ///
    /// Unlike [`MessageBanner`](super::message_banner::MessageBanner), whose global
    /// path pairs `set_global` with [`show_global`](super::message_banner::MessageBanner::show_global)
    /// (rendered lazily inside `island_central_panel`), the overlay pairs
    /// [`set_global`](Self::set_global) with `render_global`: it owns a full-window
    /// dim, input sink, and focus trap that must be painted every frame from the app
    /// loop on `Order::Foreground`, not lazily from within a panel.
    pub fn render_global(ctx: &egui::Context, secret_prompt_active: bool) {
        let mut stack = get_overlay_state(ctx);
        let Some(top) = stack.last_mut() else {
            return;
        };

        // NB: render_global does NO keyboard stripping. All key/text claiming
        // happens in `claim_input` at frame start, which the app loop gates on no
        // active secret prompt (SEC-004/F-1) — a passphrase modal rendered above
        // the overlay must keep Enter/Esc/Tab. Stripping here would be both too
        // late (end-of-frame) and ungated (would re-break the prompt). The buttoned
        // case additionally relies on the focus-lock filter set in `render_buttons`.
        let elapsed = top.created_at.elapsed();
        let stuck = stuck_reveal(elapsed);
        let show_elapsed = top.show_elapsed || stuck;
        let key = top.key;
        // Logs once on show / once per shown content change, and resets the
        // no-progress watchdog clock on real progress — a shown (description, step)
        // change OR a hidden `progress_token` advance (see `log_overlay_state`).
        log_overlay_state(top);

        // No-progress watchdog (A-1): once the topmost request has shown no
        // progress for over two minutes, escalate the reassurance copy and fire a
        // one-shot dev-error — almost always a leaked handle (C1) or an un-bounded
        // operation (C2), i.e. a bug. No panic: a time-based assert would be flaky.
        let watchdog = watchdog_tripped(top.last_progress_at);
        if watchdog && !top.watchdog_logged {
            top.watchdog_logged = true;
            tracing::error!(
                key,
                "Blocking overlay has shown no progress for over 2 minutes — \
                 likely a leaked handle or an un-bounded operation"
            );
            // TODO(SEC-001): make the no-progress watchdog actionable (auto-attach
            // an escape or enforce a frame-driven reconcile owner for button-less
            // blocks) — pending product decision; conflicts with the no-built-in-
            // cancel directive.
        }

        let dark_mode = ctx.global_style().visuals.dark_mode;
        let rect = ctx.content_rect();

        // SEC-002: the dim + pointer sink + card render on Order::Foreground so
        // they sit above Foreground popups (egui ComboBox, address autocomplete,
        // SelectionDialog) that would otherwise float over a Middle-order block and
        // stay clickable. The secret prompt is raised to match and rendered later
        // (focus-raised), so it still wins above the overlay (R-1, TC-OVL-048).
        let sink_layer =
            egui::LayerId::new(egui::Order::Foreground, egui::Id::new(OVERLAY_DIM_SINK_ID));
        ctx.layer_painter(sink_layer)
            .rect_filled(rect, 0.0, DashColors::modal_overlay());
        egui::Area::new(egui::Id::new(OVERLAY_DIM_SINK_ID))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.allocate_response(rect.size(), egui::Sense::click_and_drag());
            });

        let card_layer =
            egui::LayerId::new(egui::Order::Foreground, egui::Id::new(OVERLAY_CARD_ID));
        let mut clicked = None;
        egui::Area::new(egui::Id::new(OVERLAY_CARD_ID))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                clicked = render_card(
                    ui,
                    top,
                    dark_mode,
                    elapsed,
                    show_elapsed,
                    stuck,
                    watchdog,
                    true,
                    secret_prompt_active,
                );
            });

        // Pin the card directly above the sink. egui auto-raises any interactable
        // Area to the top of its Order on a pointer press (`area.rs` bring-to-front),
        // so a backdrop press over the sink would otherwise float it above the card
        // and bury the buttons beneath the click-absorbing sink — trapping the SPV
        // escape. A sublayer is placed above its parent after that sort each frame,
        // making the card-above-sink z-order hold by construction.
        ctx.set_sublayer(sink_layer, card_layer);

        // The click does not lower the overlay — the owning screen drains its own
        // ids via `OverlayHandle::take_actions`; the app loop only sweeps orphans.
        if let Some(action_id) = clicked {
            push_overlay_action(ctx, key, &action_id);
        }

        if show_elapsed || watchdog {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        set_overlay_state(ctx, stack);
    }
}

impl Component for ProgressOverlay {
    type DomainType = String;
    type Response = ProgressOverlayResponse;

    /// Render this instance's overlay card inline (no dim/sink — the full-window
    /// block is the global [`render_global`](ProgressOverlay::render_global)
    /// concern). Shares the [`render_card`] layout helper with the global path.
    fn show(&mut self, ui: &mut egui::Ui) -> InnerResponse<Self::Response> {
        let Some(state) = &mut self.state else {
            return empty_overlay_response(ui);
        };
        let dark_mode = ui.style().visuals.dark_mode;
        let elapsed = state.created_at.elapsed();
        let stuck = stuck_reveal(elapsed);
        let show_elapsed = state.show_elapsed || stuck;
        let watchdog = watchdog_tripped(state.last_progress_at);

        // QA-003: the instance path renders the card WITHOUT seizing global focus
        // or installing the focus-lock filter (`trap_focus = false`). That trap
        // belongs to the full-window global block; an inline, non-blocking widget
        // must leave the host screen's Tab/arrow/Esc navigation intact.
        let clicked = render_card(
            ui,
            state,
            dark_mode,
            elapsed,
            show_elapsed,
            stuck,
            watchdog,
            false,
            false,
        );
        if let Some(action_id) = &clicked {
            self.last_action = Some(action_id.clone());
        }
        let changed = clicked.is_some();

        InnerResponse::new(
            ProgressOverlayResponse {
                action: clicked,
                changed,
            },
            ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
        )
    }

    fn current_value(&self) -> Option<String> {
        self.last_action.clone()
    }
}

/// Helper for the empty-state return in [`Component::show`].
fn empty_overlay_response(ui: &mut egui::Ui) -> InnerResponse<ProgressOverlayResponse> {
    InnerResponse::new(
        ProgressOverlayResponse {
            action: None,
            changed: false,
        },
        ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
    )
}

/// Whether the topmost request has been stuck long enough to reveal the honest
/// elapsed readout and the soft reassurance line (D-4). Takes `elapsed` as a
/// parameter (the clock seam) so the threshold logic is unit-testable without a
/// real wall-clock wait.
fn stuck_reveal(elapsed: Duration) -> bool {
    elapsed >= STUCK_OVERLAY_THRESHOLD
}

/// Whether the no-progress watchdog has tripped: over [`STUCK_OVERLAY_WATCHDOG_THRESHOLD`]
/// has elapsed since the last content change (A-1). Like [`stuck_reveal`], takes
/// the measured instant as a parameter so it is unit-testable.
fn watchdog_tripped(last_progress: Instant) -> bool {
    last_progress.elapsed() >= STUCK_OVERLAY_WATCHDOG_THRESHOLD
}

/// Whether a `(current, total)` step pair is meaningful enough to render. Hides
/// nonsense pairs (`0 of 0`, `4 of 3`, `0 of 5`) rather than painting them.
fn step_is_renderable(current: u32, total: u32) -> bool {
    current >= 1 && total >= 1 && current <= total
}

/// Log the overlay once on show and once per *visible* content change (NFR-5),
/// and reset the no-progress watchdog clock on any real progress — a change in the
/// shown `(description, step)` OR an advance of the hidden `progress_token` (A-1).
///
/// The two signals are deliberately separated: the debug log fires only on a shown
/// content change (so a per-frame token advance never spams the log), while the
/// watchdog reset also honours the token (so a slow-but-advancing phase whose shown
/// copy is constant — e.g. SPV headers on a slow link — never trips a false stall).
fn log_overlay_state(state: &mut OverlayState) {
    let content = (state.description.clone(), state.step);
    if !state.logged {
        state.logged = true;
        state.logged_content = Some(content);
        state.last_progress_token = state.progress_token;
        debug!(
            description = ?state.description,
            step = ?state.step,
            "Blocking progress overlay shown"
        );
        return;
    }

    let content_changed = state.logged_content.as_ref() != Some(&content);
    let token_advanced = state.progress_token != state.last_progress_token;

    if content_changed || token_advanced {
        // Real progress (shown copy changed, or the hidden token advanced): reset
        // the no-progress watchdog clock (A-1) so a legitimately advancing flow
        // never trips it.
        state.last_progress_at = Instant::now();
        state.last_progress_token = state.progress_token;
    }

    if content_changed {
        // Only a *shown* change is logged, exactly once (NFR-5) — a per-frame token
        // advance is a hidden liveness signal, not a user-visible update.
        state.logged_content = Some(content);
        debug!(
            description = ?state.description,
            step = ?state.step,
            "Blocking progress overlay updated"
        );
    }
}

/// Render the centered card contents: spinner, optional step, optional
/// description, optional elapsed/reassurance, optional button row. Returns the
/// action id of a button clicked this frame, if any. Shared by the instance
/// [`Component::show`] and the global [`ProgressOverlay::render_global`] paths.
#[allow(clippy::too_many_arguments)]
fn render_card(
    ui: &mut egui::Ui,
    state: &mut OverlayState,
    dark_mode: bool,
    elapsed: Duration,
    show_elapsed: bool,
    stuck: bool,
    watchdog: bool,
    trap_focus: bool,
    secret_prompt_active: bool,
) -> Option<String> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(ui.style().visuals.window_fill)
        .inner_margin(egui::Margin::same(Spacing::MD as i8))
        .corner_radius(Shape::RADIUS_LG as f32)
        .shadow(Shadow::elevated())
        .stroke(egui::Stroke::new(
            Shape::BORDER_WIDTH,
            DashColors::popup_border_glow(),
        ))
        .show(ui, |ui| {
            // Clamp the card to the window so it — and its wrapped description —
            // never run off-screen in a very narrow window (FR-6 AC-6.2).
            let window_width = ui.ctx().content_rect().width();
            let max_width = CARD_MAX_WIDTH.min(window_width - 2.0 * Spacing::MD);
            ui.set_min_width(CARD_MIN_WIDTH.min(max_width));
            ui.set_max_width(max_width.max(0.0));
            ui.vertical_centered(|ui| {
                ui.add(
                    egui::Spinner::new()
                        .size(SPINNER_SIZE)
                        .color(DashColors::DASH_BLUE),
                );

                if let Some((current, total)) = state.step
                    && step_is_renderable(current, total)
                {
                    ui.add_space(Spacing::SM);
                    ui.label(
                        egui::RichText::new(format!("Step {current} of {total}"))
                            .color(DashColors::text_primary(dark_mode))
                            .strong(),
                    );
                }

                if let Some(description) = &state.description {
                    ui.add_space(Spacing::SM);
                    egui::ScrollArea::vertical()
                        .id_salt(state.key)
                        .max_height(DESCRIPTION_MAX_HEIGHT)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(description)
                                        .color(DashColors::text_primary(dark_mode)),
                                )
                                .wrap(),
                            );
                        });
                }

                if show_elapsed {
                    ui.add_space(Spacing::XS);
                    let seconds = elapsed.as_secs();
                    ui.label(
                        egui::RichText::new(format!("Elapsed: {seconds}s"))
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                }

                // The 120 s watchdog escalation replaces (never stacks with) the
                // soft 30 s reassurance line (A-1).
                if watchdog {
                    ui.add_space(Spacing::XS);
                    ui.label(
                        egui::RichText::new(STUCK_WATCHDOG_REASSURANCE)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                } else if stuck {
                    ui.add_space(Spacing::XS);
                    ui.label(
                        egui::RichText::new(STUCK_REASSURANCE)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                }

                if !state.buttons.is_empty() {
                    ui.add_space(Spacing::MD);
                    clicked =
                        render_buttons(ui, state, dark_mode, trap_focus, secret_prompt_active);
                }
            });
        });
    clicked
}

/// Render the action button row. Layout mirrors `ConfirmationDialog`: a
/// `right_to_left` row so the **primary** action hugs the RIGHT edge and any
/// **secondary** buttons sit to its LEFT; a single button hugs the right edge.
/// Within each tier, buttons render in the order they were added. Returns the
/// clicked button's action id, if any. Clicks never lower the overlay.
///
/// `trap_focus` is `true` only for the global full-window block: a button is
/// focused on raise and a focus-lock filter traps Tab/arrows/Esc on it so keyboard
/// navigation cannot escape to a widget beneath the block. The focused button is the
/// **designated keyboard escape** when the block opts into one (QA-002 refinement),
/// otherwise the first button — but the focus is purely visual: keyboard activation
/// of the escape happens at frame start in [`ProgressOverlay::claim_input`], not via
/// this focused button. `secret_prompt_active` suppresses all focus management so a
/// passphrase modal rendered above the block keeps the keyboard (SEC-001). The
/// instance [`Component`] path passes `false` for both (QA-003) so an inline,
/// non-blocking widget never seizes the host screen's focus.
fn render_buttons(
    ui: &mut egui::Ui,
    state: &mut OverlayState,
    dark_mode: bool,
    trap_focus: bool,
    secret_prompt_active: bool,
) -> Option<String> {
    let escape_action = state.keyboard_escape_action.as_deref();
    // Re-request focus every frame for an opt-in escape so a click/Tab can never
    // leave it un-focused; a non-escape block requests once and relies on the lock.
    let want_focus = trap_focus && (!state.focus_requested || escape_action.is_some());
    let mut clicked = None;
    let mut first_id = None;
    let mut escape_id = None;

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // Primaries first → rightmost (accent); secondaries after → to their left.
        let ordered = state
            .buttons
            .iter()
            .filter(|b| b.style == ButtonStyle::Primary)
            .chain(
                state
                    .buttons
                    .iter()
                    .filter(|b| b.style == ButtonStyle::Secondary),
            );
        for button in ordered {
            let response = match button.style {
                ButtonStyle::Primary => ComponentStyles::add_primary_button(ui, &button.label),
                ButtonStyle::Secondary => {
                    ComponentStyles::add_secondary_button(ui, &button.label, dark_mode)
                }
            };
            if first_id.is_none() {
                first_id = Some(response.id);
            }
            if escape_action == Some(button.action_id.as_str()) {
                escape_id = Some(response.id);
            }
            if response.clicked() {
                clicked = Some(button.action_id.clone());
            }
        }
    });

    // Pin focus to the designated escape if present, else the first button — but
    // never while a secret prompt is up: the prompt is rendered above the overlay and
    // owns the keyboard (SEC-001), and keyboard activation of the escape no longer
    // needs focus (it fires at frame start in `claim_input`).
    let focus_target = escape_id.or(first_id);
    if trap_focus
        && !secret_prompt_active
        && let Some(id) = focus_target
    {
        if want_focus {
            ui.memory_mut(|m| m.request_focus(id));
        }
        // Trap keyboard focus on the block. egui resolves Tab/arrow navigation in
        // `begin_pass` (before this code runs), so filtering those key events here
        // is too late — only a focus lock filter on the focused widget keeps
        // navigation from escaping to a widget beneath. No-op until the button has
        // held focus for a frame, which the focus request above arranges.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
        state.focus_requested = true;
    }
    clicked
}

/// Reads the overlay stack from egui context data.
fn get_overlay_state(ctx: &egui::Context) -> Vec<OverlayState> {
    ctx.data(|d| d.get_temp::<Vec<OverlayState>>(egui::Id::new(OVERLAY_STATE_ID)))
        .unwrap_or_default()
}

/// Writes the overlay stack to egui context data. Removes the slot when empty.
fn set_overlay_state(ctx: &egui::Context, stack: Vec<OverlayState>) {
    if stack.is_empty() {
        ctx.data_mut(|d| d.remove::<Vec<OverlayState>>(egui::Id::new(OVERLAY_STATE_ID)));
    } else {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(OVERLAY_STATE_ID), stack));
    }
}

/// A pending button click, scoped to the overlay entry that owns it (A-3). The
/// `key` lets the owning [`OverlayHandle`] drain only its own ids while the
/// orphan-sweeper reclaims ids whose owner is gone.
#[derive(Clone)]
struct OverlayAction {
    key: u64,
    action_id: String,
}

/// Reads the pending overlay-action queue (FIFO) from egui context data.
fn get_overlay_actions(ctx: &egui::Context) -> Vec<OverlayAction> {
    ctx.data(|d| d.get_temp::<Vec<OverlayAction>>(egui::Id::new(OVERLAY_ACTIONS_ID)))
        .unwrap_or_default()
}

/// Writes the pending overlay-action queue. Removes the slot when empty.
fn set_overlay_actions(ctx: &egui::Context, actions: Vec<OverlayAction>) {
    if actions.is_empty() {
        ctx.data_mut(|d| d.remove::<Vec<OverlayAction>>(egui::Id::new(OVERLAY_ACTIONS_ID)));
    } else {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(OVERLAY_ACTIONS_ID), actions));
    }
}

/// Appends an action id (scoped to its owning entry's `key`) to the queue.
/// Called from `render_global` on a button click.
fn push_overlay_action(ctx: &egui::Context, key: u64, action_id: &str) {
    let mut queue = get_overlay_actions(ctx);
    queue.push(OverlayAction {
        key,
        action_id: action_id.to_string(),
    });
    set_overlay_actions(ctx, queue);
}

/// Lifecycle helpers for an `Option<OverlayHandle>` screen field, mirroring
/// [`OptionBannerExt`](super::message_banner::OptionBannerExt). A dispatching
/// screen stores `op_overlay: Option<OverlayHandle>`, raises it when returning
/// the `BackendTask`, and lowers it in `display_task_result` via
/// `take_and_clear()` before AppState shows the result banner.
pub trait OptionOverlayExt {
    /// Take the handle (leaving `None`) and dismiss its overlay entry.
    fn take_and_clear(&mut self);

    /// Clear any existing overlay, raise a new one, and store the handle. The
    /// banner analogue is [`OptionBannerExt::replace`](super::message_banner::OptionBannerExt::replace),
    /// but this stays named `raise`: an inherent `Option::replace(value)` already
    /// exists and wins method resolution, so naming this `replace` would shadow it
    /// and make every `slot.replace(ctx, desc, config)` call fail to compile
    /// (arity mismatch against the inherent one-arg method).
    fn raise(&mut self, ctx: &egui::Context, description: impl fmt::Display, config: OverlayConfig);
}

impl OptionOverlayExt for Option<OverlayHandle> {
    fn take_and_clear(&mut self) {
        if let Some(handle) = self.take() {
            handle.clear();
        }
    }

    fn raise(
        &mut self,
        ctx: &egui::Context,
        description: impl fmt::Display,
        config: OverlayConfig,
    ) {
        self.take_and_clear();
        *self = Some(ProgressOverlay::set_global(ctx, description, config));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives one render pass over a bare context so `ctx.data`-level effects
    /// (log-once, focus) can be inspected without a kittest harness.
    fn render_once(ctx: &egui::Context) {
        #[allow(deprecated)]
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ProgressOverlay::render_global(ctx, false);
        });
    }

    #[test]
    fn step_is_renderable_accepts_valid_and_rejects_nonsense() {
        assert!(step_is_renderable(1, 1));
        assert!(step_is_renderable(3, 5));
        assert!(step_is_renderable(5, 5));
        assert!(!step_is_renderable(0, 0));
        assert!(!step_is_renderable(4, 3));
        assert!(!step_is_renderable(0, 5));
    }

    #[test]
    fn stuck_reveal_triggers_only_past_threshold() {
        assert!(!stuck_reveal(Duration::from_secs(0)));
        assert!(!stuck_reveal(
            STUCK_OVERLAY_THRESHOLD - Duration::from_millis(1)
        ));
        assert!(stuck_reveal(STUCK_OVERLAY_THRESHOLD));
        assert!(stuck_reveal(Duration::from_secs(60)));
    }

    #[test]
    fn show_pushes_entry_and_has_global_reports_it() {
        let ctx = egui::Context::default();
        assert!(!ProgressOverlay::has_global(&ctx));
        let handle = ProgressOverlay::set_global(&ctx, "Loading.", OverlayConfig::default());
        assert!(ProgressOverlay::has_global(&ctx));
        assert!(handle.is_active());
        assert!(handle.elapsed().is_some());
    }

    #[test]
    fn config_with_description_wins_over_argument() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global(
            &ctx,
            "",
            OverlayConfig::new().with_description("From config."),
        );
        let stack = get_overlay_state(&ctx);
        let entry = stack.iter().find(|s| s.key == handle.key).unwrap();
        assert_eq!(entry.description.as_deref(), Some("From config."));
    }

    #[test]
    fn spinner_only_has_no_text() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global_spinner_only(&ctx);
        let stack = get_overlay_state(&ctx);
        let entry = stack.iter().find(|s| s.key == handle.key).unwrap();
        assert!(entry.description.is_none());
        assert!(entry.step.is_none());
        assert!(entry.buttons.is_empty());
    }

    #[test]
    fn stale_handle_updates_are_none_and_do_not_panic() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global(&ctx, "Gone soon.", OverlayConfig::default());
        handle.clone().clear();
        assert!(handle.set_description("After clear").is_none());
        assert!(handle.set_step(1, 3).is_none());
        assert!(handle.clear_step().is_none());
        assert!(
            handle
                .with_action("Run in background", "overlay.bg")
                .is_none()
        );
        assert!(!ProgressOverlay::has_global(&ctx));
    }

    #[test]
    fn double_clear_is_a_noop() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global(&ctx, "Once.", OverlayConfig::default());
        handle.clone().clear();
        handle.clear();
        assert!(!ProgressOverlay::has_global(&ctx));
    }

    #[test]
    fn stack_renders_topmost_and_each_handle_clears_only_itself() {
        let ctx = egui::Context::default();
        let a = ProgressOverlay::set_global(&ctx, "Operation A.", OverlayConfig::default());
        let b = ProgressOverlay::set_global(&ctx, "Operation B.", OverlayConfig::default());
        assert!(a.is_active());
        assert!(b.is_active());

        let stack = get_overlay_state(&ctx);
        assert_eq!(
            stack.last().unwrap().description.as_deref(),
            Some("Operation B.")
        );

        b.clear();
        assert!(a.is_active());
        assert!(ProgressOverlay::has_global(&ctx));
        let stack = get_overlay_state(&ctx);
        assert_eq!(
            stack.last().unwrap().description.as_deref(),
            Some("Operation A.")
        );

        a.clear();
        assert!(!ProgressOverlay::has_global(&ctx));
    }

    /// A-3 — a handle drains its **own** clicks FIFO then empties, and the
    /// orphan-sweeper sees nothing while the owner is still live.
    #[test]
    fn handle_take_actions_drains_own_fifo_then_empties() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global_spinner_only(&ctx);
        assert!(handle.take_actions().is_empty());

        push_overlay_action(&ctx, handle.key, "first");
        push_overlay_action(&ctx, handle.key, "second");

        // The owner is live, so the orphan-sweeper must not touch its ids.
        assert!(ProgressOverlay::sweep_orphan_actions(&ctx).is_empty());

        assert_eq!(handle.take_actions(), vec!["first", "second"]);
        assert!(handle.take_actions().is_empty());
    }

    /// A-3 — two stacked overlays: a click keyed to B is drained only by B; A
    /// never steals it (no cross-owner theft).
    #[test]
    fn keyed_actions_isolate_owners() {
        let ctx = egui::Context::default();
        let a = ProgressOverlay::set_global(&ctx, "A.", OverlayConfig::default());
        let b = ProgressOverlay::set_global(&ctx, "B.", OverlayConfig::default());
        push_overlay_action(&ctx, b.key, "b:action");

        assert!(a.take_actions().is_empty(), "A must not see B's click");
        assert_eq!(b.take_actions(), vec!["b:action"]);
        assert!(b.take_actions().is_empty());
    }

    /// A-3 — a handle dropped without draining leaves its id reachable only via
    /// `sweep_orphan_actions`; `clear()` instead leaves nothing for the sweeper.
    #[test]
    fn orphan_sweeper_reclaims_only_dead_owner_ids() {
        let ctx = egui::Context::default();

        // Owner clears normally → its pending id is purged, sweeper finds nothing.
        let cleared = ProgressOverlay::set_global_spinner_only(&ctx);
        push_overlay_action(&ctx, cleared.key, "cleared:id");
        cleared.clear();
        assert!(ProgressOverlay::sweep_orphan_actions(&ctx).is_empty());

        // Owner dropped without draining → its id is orphaned and swept once.
        let dropped = ProgressOverlay::set_global_spinner_only(&ctx);
        let dropped_key = dropped.key;
        push_overlay_action(&ctx, dropped_key, "dropped:id");
        ProgressOverlay::clear_all_global(&ctx); // entry gone, id keyed to a dead owner
        // Re-enqueue against the now-dead key to model the drop-without-drain race.
        push_overlay_action(&ctx, dropped_key, "dropped:id");
        assert_eq!(
            ProgressOverlay::sweep_orphan_actions(&ctx),
            vec!["dropped:id"]
        );
        assert!(ProgressOverlay::sweep_orphan_actions(&ctx).is_empty());
    }

    /// SEC-007 — `clear_all_global` (network switch) drains the action queue too,
    /// so a click queued just before the switch cannot survive into the new
    /// context and be mis-dispatched.
    #[test]
    fn clear_all_global_clears_action_queue() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global_spinner_only(&ctx);
        push_overlay_action(&ctx, handle.key, "shielded:build:cancel");

        ProgressOverlay::clear_all_global(&ctx);

        assert!(!ProgressOverlay::has_global(&ctx), "state stack is cleared");
        assert!(
            ProgressOverlay::sweep_orphan_actions(&ctx).is_empty(),
            "SEC-007: the action queue must be cleared on a network switch"
        );
    }

    /// A pressed key-down `Event::Key` with no modifiers, for input tests.
    fn key_down(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// QA-001 — while a block is up, `claim_input` strips typed text and the
    /// navigation/confirm keys (Tab/Enter/Escape/Space/arrows) so nothing beneath
    /// the block observes them.
    #[test]
    fn claim_input_strips_text_and_nav_keys_when_block_active() {
        let ctx = egui::Context::default();
        ProgressOverlay::set_global_spinner_only(&ctx);

        let leaked = std::cell::Cell::new(true);
        let raw = egui::RawInput {
            events: vec![
                egui::Event::Text("hello".to_string()),
                key_down(egui::Key::Tab),
                key_down(egui::Key::Enter),
                key_down(egui::Key::Escape),
                key_down(egui::Key::Space),
                key_down(egui::Key::ArrowDown),
            ],
            ..Default::default()
        };
        #[allow(deprecated)]
        let _ = ctx.run(raw, |ctx| {
            ProgressOverlay::claim_input(ctx);
            ctx.input(|i| {
                leaked.set(i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Text(_)
                            | egui::Event::Key {
                                key: egui::Key::Tab
                                    | egui::Key::Enter
                                    | egui::Key::Escape
                                    | egui::Key::Space
                                    | egui::Key::ArrowDown,
                                pressed: true,
                                ..
                            }
                    )
                }));
            });
        });
        assert!(
            !leaked.get(),
            "claim_input must strip all text + nav/confirm key-down events while a block is up"
        );
    }

    /// QA-002 refinement — `with_keyboard_escape` records the designated escape
    /// action id on both the config (via `set_global`) and a live handle.
    #[test]
    fn with_keyboard_escape_records_action_via_config_and_handle() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global(
            &ctx,
            "Syncing.",
            OverlayConfig::new()
                .with_secondary_action("Continue in the background", "spv:escape")
                .with_keyboard_escape("spv:escape"),
        );
        let read = |key: u64| {
            get_overlay_state(&ctx)
                .into_iter()
                .find(|s| s.key == key)
                .and_then(|s| s.keyboard_escape_action)
        };
        assert_eq!(read(handle.key).as_deref(), Some("spv:escape"));

        // The handle-side mutator designates the escape on a block raised without it.
        let plain = ProgressOverlay::set_global(
            &ctx,
            "Working.",
            OverlayConfig::new().with_secondary_action("Continue", "later"),
        );
        assert!(read(plain.key).is_none());
        assert!(plain.with_keyboard_escape("later").is_some());
        assert_eq!(read(plain.key).as_deref(), Some("later"));
    }

    /// SEC-001/SEC-002 — a designated keyboard escape is activated at FRAME START:
    /// `claim_input` enqueues its action (focus-independent — no render has run, so
    /// no button is focused) and STRIPS Enter/Space so the key can never reach the
    /// focused button or a focus-independent handler beneath. The activation does not
    /// depend on, or wait for, the escape button holding focus.
    #[test]
    fn claim_input_escape_block_enqueues_action_and_strips_keys() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global(
            &ctx,
            "Syncing.",
            OverlayConfig::new()
                .with_secondary_action("Continue in the background", "spv:escape")
                .with_keyboard_escape("spv:escape"),
        );
        let leaked = std::cell::Cell::new(true);
        let raw = egui::RawInput {
            events: vec![key_down(egui::Key::Enter), key_down(egui::Key::Space)],
            ..Default::default()
        };
        #[allow(deprecated)]
        let _ = ctx.run(raw, |ctx| {
            ProgressOverlay::claim_input(ctx);
            ctx.input(|i| {
                leaked.set(i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Enter | egui::Key::Space,
                            pressed: true,
                            ..
                        }
                    )
                }));
            });
        });
        assert!(
            !leaked.get(),
            "Enter/Space are stripped at frame start — never reach the button or a widget beneath"
        );
        assert_eq!(
            handle.take_actions(),
            vec!["spv:escape".to_string()],
            "the escape action is enqueued directly at frame start, focus-independent"
        );
    }

    /// `claim_input` is a no-op when no overlay is active — it must not eat input
    /// from the rest of the app.
    #[test]
    fn claim_input_is_noop_when_idle() {
        let ctx = egui::Context::default();
        let kept = std::cell::Cell::new(false);
        let raw = egui::RawInput {
            events: vec![egui::Event::Text("hi".to_string())],
            ..Default::default()
        };
        #[allow(deprecated)]
        let _ = ctx.run(raw, |ctx| {
            ProgressOverlay::claim_input(ctx);
            ctx.input(|i| {
                kept.set(i.events.iter().any(|e| matches!(e, egui::Event::Text(_))));
            });
        });
        assert!(
            kept.get(),
            "claim_input must not strip input when no block is active"
        );
    }

    #[test]
    fn render_logs_once_then_marks_logged() {
        let ctx = egui::Context::default();
        ProgressOverlay::set_global(&ctx, "Working.", OverlayConfig::default());
        render_once(&ctx);
        let stack = get_overlay_state(&ctx);
        let entry = stack.last().unwrap();
        assert!(entry.logged);
        assert_eq!(
            entry.logged_content,
            Some((Some("Working.".to_string()), None))
        );
        // A second render with no content change keeps the marker stable.
        render_once(&ctx);
        let stack = get_overlay_state(&ctx);
        assert!(stack.last().unwrap().logged);
    }

    #[test]
    fn elapsed_counts_up_monotonically() {
        let ctx = egui::Context::default();
        let handle =
            ProgressOverlay::set_global(&ctx, "Slow.", OverlayConfig::new().with_elapsed());
        let first = handle.elapsed().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let second = handle.elapsed().unwrap();
        assert!(second >= first, "Instant-based elapsed never counts down");
    }

    #[test]
    fn option_overlay_ext_raise_swaps_entry() {
        let ctx = egui::Context::default();
        let mut slot: Option<OverlayHandle> = None;
        slot.raise(&ctx, "First.", OverlayConfig::default());
        let first_key = slot.as_ref().unwrap().key;
        slot.raise(&ctx, "Second.", OverlayConfig::default());
        let second_key = slot.as_ref().unwrap().key;
        assert_ne!(first_key, second_key);
        // Only the latest entry survives the swap.
        let stack = get_overlay_state(&ctx);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.last().unwrap().key, second_key);
        slot.take_and_clear();
        assert!(!ProgressOverlay::has_global(&ctx));
    }

    // ── Component (instance) path ───────────────────────────────────────────

    #[test]
    fn component_response_accessors_are_honest() {
        // No click: not changed, valid, no error, no value.
        let idle = ProgressOverlayResponse {
            action: None,
            changed: false,
        };
        assert!(!idle.has_changed());
        assert!(idle.is_valid());
        assert!(idle.error_message().is_none());
        assert!(idle.changed_value().is_none());

        // A click surfaces the button's action id as the changed value.
        let clicked = ProgressOverlayResponse {
            action: Some("overlay.bg".to_string()),
            changed: true,
        };
        assert!(clicked.has_changed());
        assert!(clicked.is_valid());
        assert_eq!(clicked.changed_value().as_deref(), Some("overlay.bg"));
    }

    #[test]
    fn component_show_renders_instance_and_reports_no_click() {
        let ctx = egui::Context::default();
        let mut overlay = ProgressOverlay::new()
            .with_description("Instance overlay.")
            .with_step(2, 5)
            .with_action("Run in background", "overlay.bg");
        // No interaction has happened yet.
        assert!(overlay.current_value().is_none());

        #[allow(deprecated)]
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = overlay.show(ui).inner;
                // A frame with no click is unchanged, valid, and value-free.
                assert!(!response.has_changed());
                assert!(response.is_valid());
                assert!(response.changed_value().is_none());
            });
        });

        // current_value still None — clicks are surfaced via the response and
        // recorded on the instance only when they occur.
        assert!(overlay.current_value().is_none());
    }

    /// A-1 — the no-progress watchdog trips only past its threshold (clock seam).
    #[test]
    fn watchdog_tripped_only_past_threshold() {
        let now = Instant::now();
        assert!(!watchdog_tripped(now));
        if let Some(old) =
            now.checked_sub(STUCK_OVERLAY_WATCHDOG_THRESHOLD + Duration::from_secs(1))
        {
            assert!(watchdog_tripped(old));
        }
        if let Some(recent) =
            now.checked_sub(STUCK_OVERLAY_WATCHDOG_THRESHOLD - Duration::from_secs(1))
        {
            assert!(!watchdog_tripped(recent));
        }
    }

    /// A-1 — a real content change resets the no-progress clock (so a progressing
    /// flow never trips the watchdog); no change leaves it untouched.
    #[test]
    fn log_overlay_state_bumps_progress_clock_on_content_change() {
        let mut state = OverlayState::new(1, Some("a".to_string()), &OverlayConfig::default());
        state.logged = true;
        state.logged_content = Some((Some("a".to_string()), None));
        state.last_progress_at = Instant::now()
            .checked_sub(STUCK_OVERLAY_WATCHDOG_THRESHOLD + Duration::from_secs(5))
            .expect("instant underflow");
        assert!(watchdog_tripped(state.last_progress_at));

        // Content changes → log_overlay_state bumps the clock.
        state.description = Some("b".to_string());
        log_overlay_state(&mut state);
        assert!(
            !watchdog_tripped(state.last_progress_at),
            "a content change must reset the no-progress clock"
        );

        // No change → the clock is left untouched.
        let before = state.last_progress_at;
        log_overlay_state(&mut state);
        assert_eq!(
            state.last_progress_at, before,
            "no content change must not touch the clock"
        );
    }

    /// A-1 (Item B) — an advancing hidden `progress_token` resets the no-progress
    /// clock even when the shown `(description, step)` is unchanged (a slow-but-
    /// advancing phase), but it must NOT emit a content-update log; an unchanged
    /// token (a genuine stall) leaves the clock alone so the watchdog still trips.
    #[test]
    fn log_overlay_state_token_advance_resets_clock_without_content_change() {
        let mut state = OverlayState::new(
            1,
            Some("Syncing with the Dash network.".to_string()),
            &OverlayConfig::new().with_progress_token(10),
        );
        // Prime as already-shown at token 10 (the `!logged` path records the token).
        log_overlay_state(&mut state);
        assert_eq!(state.last_progress_token, Some(10));

        // Age the clock past the watchdog with NO visible content change.
        state.last_progress_at = Instant::now()
            .checked_sub(STUCK_OVERLAY_WATCHDOG_THRESHOLD + Duration::from_secs(5))
            .expect("instant underflow");
        assert!(watchdog_tripped(state.last_progress_at));

        // Token advances (height climbed) → clock resets, watchdog cleared — with
        // the shown copy untouched.
        let logged_before = state.logged_content.clone();
        state.progress_token = Some(20);
        log_overlay_state(&mut state);
        assert!(
            !watchdog_tripped(state.last_progress_at),
            "an advancing hidden token must reset the no-progress clock"
        );
        assert_eq!(
            state.logged_content, logged_before,
            "a hidden token advance is NOT a shown content change (NFR-5)"
        );

        // Same token again (a true stall) → clock NOT reset, watchdog stays tripped.
        state.last_progress_at = Instant::now()
            .checked_sub(STUCK_OVERLAY_WATCHDOG_THRESHOLD + Duration::from_secs(5))
            .expect("instant underflow");
        let before = state.last_progress_at;
        log_overlay_state(&mut state);
        assert_eq!(
            state.last_progress_at, before,
            "an unchanged token must not reset the clock"
        );
        assert!(watchdog_tripped(state.last_progress_at));
    }

    /// A-1 — the watchdog dev-error flag flips once on render and stays set, so the
    /// error logs exactly once rather than every frame (NFR-5).
    #[test]
    fn watchdog_flag_flips_once_via_render() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::set_global_spinner_only(&ctx);
        {
            let mut stack = get_overlay_state(&ctx);
            let top = stack.iter_mut().find(|s| s.key == handle.key).unwrap();
            top.last_progress_at = Instant::now()
                .checked_sub(STUCK_OVERLAY_WATCHDOG_THRESHOLD + Duration::from_secs(5))
                .expect("instant underflow");
            set_overlay_state(&ctx, stack);
        }
        let logged = |ctx: &egui::Context| {
            get_overlay_state(ctx)
                .iter()
                .find(|s| s.key == handle.key)
                .map(|s| s.watchdog_logged)
                .unwrap()
        };
        render_once(&ctx);
        assert!(logged(&ctx), "the watchdog flag flips on render");
        render_once(&ctx);
        assert!(logged(&ctx), "and stays set across frames");
    }

    /// QA-007 — the instance `clear()` makes the empty-response path reachable via
    /// the public API: after clear, `show()` renders nothing and reports no value.
    #[test]
    fn instance_clear_reaches_empty_response() {
        let ctx = egui::Context::default();
        let mut overlay = ProgressOverlay::new().with_description("Working.");
        overlay.clear();
        #[allow(deprecated)]
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = overlay.show(ui).inner;
                assert!(!response.has_changed());
                assert!(response.changed_value().is_none());
            });
        });
        assert!(overlay.current_value().is_none());
    }
}
