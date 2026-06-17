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
//! with [`OverlayConfig::with_button`] / [`OverlayHandle::with_button`], picking
//! its own opaque action id and label. Clicking the button enqueues that action
//! id; the overlay does **not** auto-lower. The owning screen drains action ids
//! via [`ProgressOverlay::take_actions`] (FIFO) and decides what to do —
//! including running its own cancellation logic if it chose to label a button
//! "Cancel".
//!
//! ## Two render paths (mirrors `MessageBanner`)
//!
//! Like `MessageBanner`, this type has both an instance [`Component`] path and a
//! global path, sharing one layout helper ([`render_card`]):
//!
//! - **Global** — state lives in egui `ctx.data`; [`ProgressOverlay::show_global`]
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

/// Diameter of the indeterminate spinner inside the card.
const SPINNER_SIZE: f32 = 32.0;
/// Card minimum width so short content still reads as a deliberate dialog.
const CARD_MIN_WIDTH: f32 = 240.0;
/// Card maximum width so long descriptions wrap instead of stretching.
const CARD_MAX_WIDTH: f32 = 420.0;
/// Description scrolls inside the card past this height (FR-6: never off-screen).
const DESCRIPTION_MAX_HEIGHT: f32 = 160.0;

/// Reassurance line revealed once the stuck threshold passes.
const STUCK_REASSURANCE: &str = "This is taking longer than usual.";

/// Monotonic counter for generating unique overlay keys.
static OVERLAY_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_overlay_key() -> u64 {
    // Relaxed is sufficient: we only need uniqueness, not ordering. The counter
    // runs in a single-threaded UI context.
    OVERLAY_KEY_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// One generic action button on the overlay card. The caller owns both the
/// `label` (an i18n unit) and the `action_id` enqueued on click.
#[derive(Clone)]
struct OverlayButton {
    label: String,
    action_id: String,
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
    /// Set once focus has been placed on the first button (focus trap).
    focus_requested: bool,
}

impl OverlayState {
    fn new(key: u64, description: Option<String>, config: &OverlayConfig) -> Self {
        Self {
            key,
            description,
            step: config.step,
            buttons: config.buttons.clone(),
            show_elapsed: config.show_elapsed,
            created_at: Instant::now(),
            logged: false,
            logged_content: None,
            focus_requested: false,
        }
    }
}

/// Builder/config for [`ProgressOverlay::show_global`]. `OverlayConfig::default()`
/// is a spinner-only block: no counter, no buttons, elapsed off.
#[derive(Clone, Default)]
pub struct OverlayConfig {
    description: Option<String>,
    step: Option<(u32, u32)>,
    show_elapsed: bool,
    buttons: Vec<OverlayButton>,
}

impl OverlayConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the description. The `description` argument of `show_global` wins when
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

    /// Add a generic action button. The caller owns the opaque `id` enqueued on
    /// click and the `label` shown on the button; clicking does not lower the
    /// overlay — the owning screen drains the id and decides what to do.
    pub fn with_button(mut self, id: impl fmt::Display, label: impl fmt::Display) -> Self {
        self.buttons.push(OverlayButton {
            label: label.to_string(),
            action_id: id.to_string(),
        });
        self
    }
}

/// Lifecycle handle for a raised overlay, returned by
/// [`ProgressOverlay::show_global`]. Identifies its entry by an internal key, so
/// content can be updated without losing the reference. Methods are no-ops
/// returning `None` once the entry is gone.
///
/// INTENTIONAL(SEC-004): `OverlayHandle` is Send+Sync because `egui::Context` is
/// Send+Sync with internal locking. Acceptable for a single-threaded UI app;
/// egui's own thread-safety guarantees apply.
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

    /// Attach a generic action button — opaque `id` enqueued on click + `label`
    /// shown verbatim. Returns `None` if the entry is gone.
    pub fn with_button(&self, id: impl fmt::Display, label: impl fmt::Display) -> Option<&Self> {
        let action_id = id.to_string();
        let label = label.to_string();
        self.mutate(|s| {
            s.buttons.push(OverlayButton { label, action_id });
        })
    }

    /// Dismiss only this handle's entry. The overlay lowers when the stack empties.
    pub fn clear(self) {
        let mut stack = get_overlay_state(&self.ctx);
        let before = stack.len();
        stack.retain(|s| s.key != self.key);
        if stack.len() != before {
            debug!(key = self.key, "Blocking progress overlay dismissed");
        }
        set_overlay_state(&self.ctx, stack);
    }

    /// Find this handle's entry, apply `f`, and write the stack back. Resets the
    /// content-log marker so the next render logs the change once.
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
/// the global `ctx.data` path ([`show_global`](Self::show_global) /
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

    /// Add a generic action button. The caller owns the opaque `id` surfaced
    /// through [`ProgressOverlayResponse`] on click and the `label` shown on it.
    pub fn with_button(mut self, id: impl fmt::Display, label: impl fmt::Display) -> Self {
        if let Some(state) = &mut self.state {
            state.buttons.push(OverlayButton {
                label: label.to_string(),
                action_id: id.to_string(),
            });
        }
        self
    }

    // ── Global (ctx.data) path ──────────────────────────────────────────────

    /// Raise the overlay and return its handle. Non-blocking: only writes
    /// `ctx.data`. The `description` argument is used unless `config` already
    /// carries one.
    pub fn show_global(
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
    pub fn show_global_spinner_only(ctx: &egui::Context) -> OverlayHandle {
        Self::show_global(ctx, "", OverlayConfig::default())
    }

    /// Whether any overlay is active. Cheap one-slot read (NFR-6).
    pub fn has_global(ctx: &egui::Context) -> bool {
        !get_overlay_state(ctx).is_empty()
    }

    /// Drain the action-id queue (FIFO) for the app loop to dispatch.
    pub fn take_actions(ctx: &egui::Context) -> Vec<String> {
        let actions = get_overlay_actions(ctx);
        if !actions.is_empty() {
            set_overlay_actions(ctx, Vec::new());
        }
        actions
    }

    /// Clear every entry — used on network switch alongside the banner reset.
    pub fn clear_all_global(ctx: &egui::Context) {
        set_overlay_state(ctx, Vec::new());
    }

    /// Render the topmost entry. Call once per frame from `AppState::update`,
    /// after the panels and before the secret prompt. Early-outs to a single
    /// `ctx.data` read when no overlay is active (NFR-6).
    pub fn render_global(ctx: &egui::Context) {
        let mut stack = get_overlay_state(ctx);
        let Some(top) = stack.last_mut() else {
            return;
        };

        // Full input block, scoped to the overlay-active branch so global
        // shortcuts are untouched when idle. Esc/Tab/Enter are swallowed so they
        // cannot dismiss the overlay, move focus to a widget beneath it, or
        // activate a focused button. The overlay never implicitly dismisses —
        // only a handle clear lowers it.
        ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            i.events.retain(|e| {
                !matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::Tab | egui::Key::Enter,
                        pressed: true,
                        ..
                    }
                )
            });
        });

        let elapsed = top.created_at.elapsed();
        let stuck = stuck_reveal(elapsed);
        let show_elapsed = top.show_elapsed || stuck;
        log_overlay_state(top);

        let dark_mode = ctx.style().visuals.dark_mode;
        let rect = ctx.content_rect();

        // Dim + pointer sink share one Middle-order layer so the dim is always
        // behind the card (a later Middle area). The sink consumes pointer
        // events; its own clicks are ignored, so a backdrop click never dismisses.
        let sink_layer =
            egui::LayerId::new(egui::Order::Middle, egui::Id::new(OVERLAY_DIM_SINK_ID));
        ctx.layer_painter(sink_layer)
            .rect_filled(rect, 0.0, DashColors::modal_overlay());
        egui::Area::new(egui::Id::new(OVERLAY_DIM_SINK_ID))
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.allocate_response(rect.size(), egui::Sense::click_and_drag());
            });

        let mut clicked = None;
        egui::Area::new(egui::Id::new(OVERLAY_CARD_ID))
            .order(egui::Order::Middle)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                clicked = render_card(ui, top, dark_mode, elapsed, show_elapsed, stuck);
            });

        // The click does not lower the overlay — the app loop drains the queue
        // via `take_actions` and the owning screen decides what to do.
        if let Some(action_id) = clicked {
            push_overlay_action(ctx, &action_id);
        }

        if show_elapsed {
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
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let elapsed = state.created_at.elapsed();
        let stuck = stuck_reveal(elapsed);
        let show_elapsed = state.show_elapsed || stuck;

        let clicked = render_card(ui, state, dark_mode, elapsed, show_elapsed, stuck);
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
/// elapsed readout and reassurance line (D-4).
fn stuck_reveal(elapsed: Duration) -> bool {
    elapsed >= STUCK_OVERLAY_THRESHOLD
}

/// Whether a `(current, total)` step pair is meaningful enough to render. Hides
/// nonsense pairs (`0 of 0`, `4 of 3`, `0 of 5`) rather than painting them.
fn step_is_renderable(current: u32, total: u32) -> bool {
    current >= 1 && total >= 1 && current <= total
}

/// Log the overlay once on show and once per content change (NFR-5).
fn log_overlay_state(state: &mut OverlayState) {
    let content = (state.description.clone(), state.step);
    if !state.logged {
        state.logged = true;
        state.logged_content = Some(content);
        debug!(
            description = ?state.description,
            step = ?state.step,
            "Blocking progress overlay shown"
        );
    } else if state.logged_content.as_ref() != Some(&content) {
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
fn render_card(
    ui: &mut egui::Ui,
    state: &mut OverlayState,
    dark_mode: bool,
    elapsed: Duration,
    show_elapsed: bool,
    stuck: bool,
) -> Option<String> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(ui.ctx().style().visuals.window_fill)
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
                    ui.label(
                        egui::RichText::new(format!("Elapsed: {}s", elapsed.as_secs()))
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                }

                if stuck {
                    ui.add_space(Spacing::XS);
                    ui.label(
                        egui::RichText::new(STUCK_REASSURANCE)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                }

                if !state.buttons.is_empty() {
                    ui.add_space(Spacing::MD);
                    clicked = render_buttons(ui, state);
                }
            });
        });
    clicked
}

/// Render the generic button row left-to-right in insertion order. Returns the
/// clicked button's action id, if any. The first button is the focus stop on
/// raise, and a focus lock filter traps Tab/arrows/Esc on it so keyboard
/// navigation cannot escape to a widget beneath the block. Clicks never lower
/// the overlay.
fn render_buttons(ui: &mut egui::Ui, state: &mut OverlayState) -> Option<String> {
    let want_focus = !state.focus_requested;
    let mut clicked = None;
    let mut focus_stop = None;

    ui.horizontal(|ui| {
        for button in &state.buttons {
            let response = ComponentStyles::add_primary_button(ui, &button.label);
            if focus_stop.is_none() {
                focus_stop = Some(response.id);
                if want_focus {
                    response.request_focus();
                }
            }
            if response.clicked() {
                clicked = Some(button.action_id.clone());
            }
        }
    });

    if let Some(id) = focus_stop {
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

/// Reads the pending overlay-action queue (FIFO) from egui context data.
fn get_overlay_actions(ctx: &egui::Context) -> Vec<String> {
    ctx.data(|d| d.get_temp::<Vec<String>>(egui::Id::new(OVERLAY_ACTIONS_ID)))
        .unwrap_or_default()
}

/// Writes the pending overlay-action queue. Removes the slot when empty.
fn set_overlay_actions(ctx: &egui::Context, actions: Vec<String>) {
    if actions.is_empty() {
        ctx.data_mut(|d| d.remove::<Vec<String>>(egui::Id::new(OVERLAY_ACTIONS_ID)));
    } else {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(OVERLAY_ACTIONS_ID), actions));
    }
}

/// Appends an action id to the queue. Called from the renderer on a button click.
fn push_overlay_action(ctx: &egui::Context, action_id: &str) {
    let mut queue = get_overlay_actions(ctx);
    queue.push(action_id.to_string());
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

    /// Clear any existing overlay, raise a new one, and store the handle. Named
    /// `raise` (not `replace`) to avoid shadowing the inherent `Option::replace`.
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
        *self = Some(ProgressOverlay::show_global(ctx, description, config));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives one render pass over a bare context so `ctx.data`-level effects
    /// (log-once, focus) can be inspected without a kittest harness.
    fn render_once(ctx: &egui::Context) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ProgressOverlay::render_global(ctx);
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
        let handle = ProgressOverlay::show_global(&ctx, "Loading.", OverlayConfig::default());
        assert!(ProgressOverlay::has_global(&ctx));
        assert!(handle.is_active());
        assert!(handle.elapsed().is_some());
    }

    #[test]
    fn config_with_description_wins_over_argument() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::show_global(
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
        let handle = ProgressOverlay::show_global_spinner_only(&ctx);
        let stack = get_overlay_state(&ctx);
        let entry = stack.iter().find(|s| s.key == handle.key).unwrap();
        assert!(entry.description.is_none());
        assert!(entry.step.is_none());
        assert!(entry.buttons.is_empty());
    }

    #[test]
    fn stale_handle_updates_are_none_and_do_not_panic() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::show_global(&ctx, "Gone soon.", OverlayConfig::default());
        handle.clone().clear();
        assert!(handle.set_description("After clear").is_none());
        assert!(handle.set_step(1, 3).is_none());
        assert!(handle.clear_step().is_none());
        assert!(
            handle
                .with_button("overlay.bg", "Run in background")
                .is_none()
        );
        assert!(!ProgressOverlay::has_global(&ctx));
    }

    #[test]
    fn double_clear_is_a_noop() {
        let ctx = egui::Context::default();
        let handle = ProgressOverlay::show_global(&ctx, "Once.", OverlayConfig::default());
        handle.clone().clear();
        handle.clear();
        assert!(!ProgressOverlay::has_global(&ctx));
    }

    #[test]
    fn stack_renders_topmost_and_each_handle_clears_only_itself() {
        let ctx = egui::Context::default();
        let a = ProgressOverlay::show_global(&ctx, "Operation A.", OverlayConfig::default());
        let b = ProgressOverlay::show_global(&ctx, "Operation B.", OverlayConfig::default());
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

    #[test]
    fn take_actions_drains_fifo_then_empties() {
        let ctx = egui::Context::default();
        assert!(ProgressOverlay::take_actions(&ctx).is_empty());
        push_overlay_action(&ctx, "first");
        push_overlay_action(&ctx, "second");
        assert_eq!(ProgressOverlay::take_actions(&ctx), vec!["first", "second"]);
        assert!(ProgressOverlay::take_actions(&ctx).is_empty());
    }

    #[test]
    fn render_logs_once_then_marks_logged() {
        let ctx = egui::Context::default();
        ProgressOverlay::show_global(&ctx, "Working.", OverlayConfig::default());
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
            ProgressOverlay::show_global(&ctx, "Slow.", OverlayConfig::new().with_elapsed());
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
            .with_button("overlay.bg", "Run in background");
        // No interaction has happened yet.
        assert!(overlay.current_value().is_none());

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
}
