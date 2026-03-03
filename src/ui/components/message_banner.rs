use crate::ui::MessageType;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{DashColors, Shape, Spacing, Typography};
use egui::InnerResponse;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, error, warn};

const DEFAULT_AUTO_DISMISS: Duration = Duration::from_secs(5);
const MAX_BANNERS: usize = 5;
const BANNER_STATE_ID: &str = "__global_message_banner";
/// Maximum height for the expanded details section before scrolling.
const DETAILS_MAX_HEIGHT: f32 = 120.0;

/// Monotonic counter for generating unique banner keys.
static BANNER_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_banner_key() -> u64 {
    // Relaxed is sufficient: we only need uniqueness (monotonic counter),
    // not ordering with other atomic operations. The counter runs in a
    // single-threaded UI context.
    BANNER_KEY_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// The domain type for `MessageBanner`, representing the banner's lifecycle state.
#[derive(Clone, Debug, PartialEq)]
pub enum BannerStatus {
    /// The banner is currently visible.
    Visible,
    /// The user clicked the dismiss button.
    Dismissed,
    /// The banner expired via auto-dismiss.
    TimedOut,
}

/// Response returned by `MessageBanner::show()` via the `Component` trait.
#[derive(Clone)]
pub struct MessageBannerResponse {
    pub status: Option<BannerStatus>,
    changed: bool,
}

impl ComponentResponse for MessageBannerResponse {
    type DomainType = BannerStatus;

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.status
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

#[derive(Clone)]
struct BannerState {
    key: u64,
    text: String,
    message_type: MessageType,
    created_at: Instant,
    /// When `Some`, the banner auto-dismisses after this duration.
    /// `None` means the banner persists until manually dismissed.
    auto_dismiss_after: Option<Duration>,
    /// When `true`, display elapsed time since creation instead of countdown.
    show_elapsed: bool,
    /// Optional technical details (shown in a collapsible section).
    details: Option<String>,
    /// Optional recovery suggestion (shown inline below the summary).
    suggestion: Option<String>,
    /// Whether the details section is currently expanded.
    details_expanded: bool,
    /// Whether the banner has been logged (to avoid duplicate log entries on each frame).
    logged: bool,
}

impl BannerState {
    /// Create a fresh banner with a new key and default auto-dismiss for the given type.
    fn new(key: u64, text: String, message_type: MessageType) -> Self {
        Self {
            key,
            text,
            message_type,
            created_at: Instant::now(),
            auto_dismiss_after: default_auto_dismiss(message_type),
            show_elapsed: false,
            details: None,
            suggestion: None,
            details_expanded: false,
            logged: false,
        }
    }

    /// Reset an existing banner's content, keeping its key.
    /// Resets timestamps, clears details/suggestion, resets logged flag.
    fn reset_to(&mut self, text: String, message_type: MessageType) {
        self.text = text;
        self.message_type = message_type;
        self.created_at = Instant::now();
        self.auto_dismiss_after = default_auto_dismiss(message_type);
        self.show_elapsed = false;
        self.details = None;
        self.suggestion = None;
        self.details_expanded = false;
        self.logged = false;
    }

    /// Emits a tracing log for this banner, with log level based on message type.
    fn log(&self) {
        let text = self.text.as_str();
        let details = self.details.as_deref();
        match self.message_type {
            MessageType::Error => error!(banner = text, details, "Banner displayed"),
            MessageType::Warning => warn!(banner = text, details, "Banner displayed"),
            MessageType::Success | MessageType::Info => {
                debug!(banner = text, details, "Banner displayed")
            }
        }
    }
}

/// Handle for a global banner, returned by [`MessageBanner::set_global`] and
/// [`MessageBanner::replace_global`]. Identifies the banner by an internal key,
/// so the display text can be updated without losing the reference.
///
/// The handle is `'static` and safe to store. Methods that modify the banner
/// (`set_message`, `with_auto_dismiss`) take `&self` so the handle can be reused.
///
/// INTENTIONAL(SEC-004): BannerHandle is Send+Sync because egui::Context is
/// Send+Sync with internal locking. This is acceptable for a single-threaded
/// UI app; egui's own thread-safety guarantees apply.
#[derive(Clone)]
pub struct BannerHandle {
    ctx: egui::Context,
    key: u64,
}

impl BannerHandle {
    /// Returns how long ago this banner was created, looked up from context data.
    /// Returns `None` if the banner no longer exists.
    pub fn elapsed(&self) -> Option<Duration> {
        let banners = get_banners(&self.ctx);
        banners
            .iter()
            .find(|b| b.key == self.key)
            .map(|b| b.created_at.elapsed())
    }

    /// Update the display text of this banner.
    /// Returns `None` if the banner no longer exists.
    pub fn set_message(&self, text: impl fmt::Display) -> Option<&Self> {
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.text = text.to_string();
        set_banners(&self.ctx, banners);
        Some(self)
    }

    /// Override the auto-dismiss duration for this banner.
    /// Resets the countdown timer.
    /// Returns `None` if the banner no longer exists.
    pub fn with_auto_dismiss(&self, duration: Duration) -> Option<&Self> {
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.auto_dismiss_after = Some(duration);
        b.created_at = Instant::now();
        set_banners(&self.ctx, banners);
        Some(self)
    }

    /// Enable elapsed-time display on this banner. Disables auto-dismiss
    /// and shows how long the banner has been visible (e.g. for long-running operations).
    /// Returns `None` if the banner no longer exists.
    pub fn with_elapsed(&self) -> Option<&Self> {
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.show_elapsed = true;
        b.auto_dismiss_after = None;
        set_banners(&self.ctx, banners);
        Some(self)
    }

    /// Attach optional technical details to this banner.
    /// Details are shown in a collapsible section (collapsed by default).
    ///
    /// Accepts `impl Debug` (not `Display`) because callers typically pass
    /// error types whose `Debug` representation includes structured context
    /// (nested causes, variant names) that is more useful in a diagnostic
    /// details pane than the single-line `Display` output.
    ///
    /// INTENTIONAL(RUST-003): When plain strings are passed, `{:?}` wraps them
    /// in quotes. This is acceptable since `with_details` is primarily for
    /// error types, not user-facing text.
    ///
    /// Returns `None` if the banner no longer exists.
    pub fn with_details(&self, details: impl fmt::Debug) -> Option<&Self> {
        let details = format!("{:?}", details);
        if details.is_empty() {
            return Some(self);
        }
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.details = Some(details);
        set_banners(&self.ctx, banners);
        Some(self)
    }

    /// Attach an optional recovery suggestion to this banner.
    /// The suggestion is shown inline (visible without expanding).
    /// Returns `None` if the banner no longer exists.
    pub fn with_suggestion(&self, suggestion: impl fmt::Display) -> Option<&Self> {
        let suggestion = suggestion.to_string();
        if suggestion.is_empty() {
            return Some(self);
        }
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.suggestion = Some(suggestion);
        set_banners(&self.ctx, banners);
        Some(self)
    }

    /// Remove this banner immediately.
    pub fn clear(self) {
        let mut banners = get_banners(&self.ctx);
        banners.retain(|b| b.key != self.key);
        set_banners(&self.ctx, banners);
    }
}

/// A banner widget for displaying screen-level messages.
///
/// Supports two modes:
/// - **Global**: State stored in egui context data, rendered by `island_central_panel`.
///   Use `set_global`, `clear_global_message`, `show_global`, `has_global`.
/// - **Per-instance**: Each screen owns a `MessageBanner` and calls `show()`.
///
/// Follows component conventions: private fields, `new()` constructor, builder methods.
pub struct MessageBanner {
    state: Option<BannerState>,
}

impl MessageBanner {
    /// Creates an empty banner (no message displayed).
    pub fn new() -> Self {
        Self { state: None }
    }

    /// Sets or replaces the current message. Resets the auto-dismiss timer.
    /// An empty string is treated as a clear operation.
    pub fn set_message(&mut self, text: impl fmt::Display, message_type: MessageType) -> &mut Self {
        let text = text.to_string();
        if text.is_empty() {
            self.state = None;
        } else {
            self.state = Some(BannerState::new(next_banner_key(), text, message_type));
        }
        self
    }

    /// Override the auto-dismiss duration for the current message.
    /// Resets the countdown timer. No-op if no message is set.
    #[allow(dead_code)]
    pub fn set_auto_dismiss(&mut self, duration: Duration) -> &mut Self {
        if let Some(state) = &mut self.state {
            state.auto_dismiss_after = Some(duration);
            state.created_at = Instant::now();
        }
        self
    }

    /// Clears the current message immediately.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.state = None;
    }

    /// Returns whether a message is currently displayed.
    pub fn has_message(&self) -> bool {
        self.state.is_some()
    }

    // -- Global API (egui context data) --
    //
    // Multiple messages can be displayed simultaneously. Messages are
    // deduplicated by text — calling `set_global` with the same text twice
    // results in a single banner. Use `replace_global` to swap one message
    // for another (e.g., replacing a generic "Success" with a specific one).

    /// Adds a global banner message if one with the same text does not already exist.
    ///
    /// **Idempotent**: if a banner with identical text is already displayed,
    /// this is a no-op and the existing banner is returned unchanged
    /// (timestamps, auto-dismiss timer, and `logged` flag are all preserved).
    /// This makes it safe to call every frame without side-effects.
    ///
    /// To reset the auto-dismiss timer of an existing banner, use
    /// [`replace_global`](Self::replace_global) with the same text for both
    /// `old_text` and `new_text`, or store the returned [`BannerHandle`] and
    /// call [`BannerHandle::with_auto_dismiss`].
    ///
    /// Evicts the oldest message when the cap ([`MAX_BANNERS`]) is reached.
    ///
    /// Returns a [`BannerHandle`] for updating or clearing the banner later.
    pub fn set_global(
        ctx: &egui::Context,
        text: impl fmt::Display,
        message_type: MessageType,
    ) -> BannerHandle {
        let text = text.to_string();
        let mut banners = get_banners(ctx);
        if let Some(existing) = banners.iter_mut().find(|b| b.text == text) {
            // Same text already displayed: update message_type if it changed,
            // but preserve timestamps and auto-dismiss timer (idempotent for text).
            if existing.message_type != message_type {
                existing.message_type = message_type;
                let key = existing.key;
                set_banners(ctx, banners);
                return BannerHandle {
                    ctx: ctx.clone(),
                    key,
                };
            }
            return BannerHandle {
                ctx: ctx.clone(),
                key: existing.key,
            };
        }
        let key = next_banner_key();
        if !text.is_empty() {
            banners.push(BannerState::new(key, text, message_type));
            if banners.len() > MAX_BANNERS {
                let evicted = banners.remove(0);
                warn!(
                    "Banner evicted (capacity {}): {:?}",
                    MAX_BANNERS, evicted.message_type,
                );
            }
            set_banners(ctx, banners);
        }
        BannerHandle {
            ctx: ctx.clone(),
            key,
        }
    }

    /// Finds a message by `old_text` and replaces it with `new_text`.
    /// If `old_text` is not found, falls back to adding `new_text` as a new
    /// message (with dedup check). This fallback is intentional: callers use
    /// `replace_global` for progress updates where the previous banner may
    /// have been dismissed or evicted, and the new message should still appear.
    ///
    /// If `old_text` is not found but `new_text` is already displayed, returns
    /// a handle to the existing banner without resetting it (consistent with
    /// [`Self::set_global`] idempotency).
    ///
    /// **Empty `new_text`**: clears the `old_text` banner (if present) and
    /// returns a handle with a fresh key that does not correspond to any banner.
    /// Subsequent calls on this handle (`set_message`, `with_details`, `clear`)
    /// are safe no-ops returning `None`.
    ///
    /// Returns a [`BannerHandle`] for updating or clearing the banner later.
    pub fn replace_global(
        ctx: &egui::Context,
        old_text: impl fmt::Display,
        new_text: impl fmt::Display,
        message_type: MessageType,
    ) -> BannerHandle {
        let old_text = old_text.to_string();
        let new_text = new_text.to_string();
        if new_text.is_empty() {
            Self::clear_global_message(ctx, &old_text);
            return BannerHandle {
                ctx: ctx.clone(),
                key: next_banner_key(),
            };
        }
        let mut banners = get_banners(ctx);
        let key;
        if let Some(b) = banners.iter_mut().find(|b| b.text == old_text) {
            key = b.key;
            b.reset_to(new_text, message_type);
        } else if let Some(existing) = banners.iter().find(|b| b.text == new_text) {
            // Idempotent: if new_text already displayed, return handle without
            // resetting (consistent with set_global behavior).
            key = existing.key;
        } else {
            key = next_banner_key();
            banners.push(BannerState::new(key, new_text, message_type));
            if banners.len() > MAX_BANNERS {
                let evicted = banners.remove(0);
                warn!(
                    "Banner evicted (capacity {}): {:?}",
                    MAX_BANNERS, evicted.message_type,
                );
            }
        }
        set_banners(ctx, banners);
        BannerHandle {
            ctx: ctx.clone(),
            key,
        }
    }

    /// Clears the specific global banner message matching `text`.
    pub fn clear_global_message(ctx: &egui::Context, text: impl fmt::Display) {
        let text = text.to_string();
        let mut banners = get_banners(ctx);
        banners.retain(|b| b.text != text);
        set_banners(ctx, banners);
    }

    /// Clears all global banner messages.
    ///
    /// Use when the context changes significantly (e.g., network switch) and
    /// stale messages from the previous context should not persist.
    pub fn clear_all_global(ctx: &egui::Context) {
        set_banners(ctx, vec![]);
    }

    /// Returns whether any global banner messages exist.
    #[allow(dead_code)]
    pub fn has_global(ctx: &egui::Context) -> bool {
        !get_banners(ctx).is_empty()
    }

    /// Renders all global banners from egui context data.
    /// Call inside `island_central_panel` before content.
    pub fn show_global(ui: &mut egui::Ui) {
        let mut banners = get_banners(ui.ctx());
        if banners.is_empty() {
            return;
        }
        // Always write back: process_banner() mutates state (auto-dismiss timers,
        // expanded flags) even when no banners are removed.
        banners.retain_mut(|b| process_banner(ui, b) == BannerStatus::Visible);
        set_banners(ui.ctx(), banners);
    }
}

impl Default for MessageBanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for MessageBanner {
    type DomainType = BannerStatus;
    type Response = MessageBannerResponse;

    fn show(&mut self, ui: &mut egui::Ui) -> InnerResponse<Self::Response> {
        let Some(state) = &mut self.state else {
            return empty_response(ui);
        };
        let status = process_banner(ui, state);
        if status != BannerStatus::Visible {
            self.state = None;
        }
        let changed = status != BannerStatus::Visible;
        InnerResponse::new(
            MessageBannerResponse {
                status: Some(status),
                changed,
            },
            ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
        )
    }

    fn current_value(&self) -> Option<BannerStatus> {
        if self.state.is_some() {
            Some(BannerStatus::Visible)
        } else {
            None
        }
    }
}

/// Returns the default auto-dismiss duration for a message type.
/// `Success` and `Info` auto-dismiss; `Error` and `Warning` persist.
fn default_auto_dismiss(message_type: MessageType) -> Option<Duration> {
    match message_type {
        MessageType::Success | MessageType::Info => Some(DEFAULT_AUTO_DISMISS),
        MessageType::Error | MessageType::Warning => None,
    }
}

/// Helper for the empty-state return in `Component::show()`.
fn empty_response(ui: &mut egui::Ui) -> InnerResponse<MessageBannerResponse> {
    InnerResponse::new(
        MessageBannerResponse {
            status: None,
            changed: false,
        },
        ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
    )
}

/// Processes a single banner: checks expiry, renders, handles dismiss, requests repaint.
/// Returns the banner's resulting status.
fn process_banner(ui: &mut egui::Ui, state: &mut BannerState) -> BannerStatus {
    let elapsed = state.created_at.elapsed();

    // Check auto-dismiss expiry
    if let Some(duration) = state.auto_dismiss_after
        && elapsed >= duration
    {
        return BannerStatus::TimedOut;
    }

    // Compute the right-side time annotation
    let annotation = if state.show_elapsed {
        Some(format!("({}s)", elapsed.as_secs()))
    } else if let Some(duration) = state.auto_dismiss_after {
        let remaining = duration.saturating_sub(elapsed);
        Some(format!("({}s)", remaining.as_secs() + 1))
    } else {
        None
    };

    // Log banner message once on first display
    if !state.logged {
        state.logged = true;
        state.log();
    }

    if render_banner(
        ui,
        &state.text,
        state.message_type,
        annotation.as_deref(),
        state.suggestion.as_deref(),
        state.details.as_deref(),
        &mut state.details_expanded,
    ) {
        return BannerStatus::Dismissed;
    }
    if state.auto_dismiss_after.is_some() || state.show_elapsed {
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }
    BannerStatus::Visible
}

/// Shared rendering logic for both global and per-instance banners.
/// Returns `true` if the dismiss button was clicked.
fn render_banner(
    ui: &mut egui::Ui,
    text: &str,
    message_type: MessageType,
    annotation: Option<&str>,
    suggestion: Option<&str>,
    details: Option<&str>,
    details_expanded: &mut bool,
) -> bool {
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    let fg_color = DashColors::message_color(message_type, dark_mode);
    let bg_color = DashColors::message_background_color(message_type, dark_mode);
    let secondary_color = DashColors::text_secondary(dark_mode);

    let icon = icon_for_type(message_type);
    let mut dismissed = false;

    egui::Frame::new()
        .fill(bg_color)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(Shape::RADIUS_SM as f32)
        .stroke(egui::Stroke::new(Shape::BORDER_WIDTH, fg_color))
        .show(ui, |ui| {
            // First row: icon + wrapping text + right-aligned dismiss
            let available_width = ui.available_width();

            ui.horizontal_top(|ui| {
                // Icon
                ui.label(egui::RichText::new(icon).color(fg_color).strong());
                ui.add_space(Spacing::XS);

                // Reserve space for dismiss button and annotation on the right
                let dismiss_width = 40.0;
                let annotation_width = if let Some(ann) = annotation {
                    // Annotations are short digit strings like "(5s)", "(30s)".
                    // Average character width ~0.4× font size for digits/parens.
                    let char_width = Typography::SCALE_SM * 0.4;
                    ann.len() as f32 * char_width + ui.spacing().item_spacing.x
                } else {
                    0.0
                };
                let text_width =
                    (available_width - dismiss_width - annotation_width - 30.0).max(0.0);

                // Message text with wrapping, left-aligned
                ui.allocate_ui(egui::vec2(text_width, 0.0), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(text).color(fg_color)).wrap());
                });

                // Right-aligned: annotation + dismiss
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    let dismiss_response = ui.add(
                        egui::Label::new(
                            egui::RichText::new("\u{274C}")
                                .color(fg_color)
                                .font(Typography::body_small()),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if dismiss_response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if dismiss_response.clicked() {
                        dismissed = true;
                    }

                    if let Some(annotation) = annotation {
                        ui.label(
                            egui::RichText::new(annotation)
                                .font(Typography::body_small())
                                .color(secondary_color),
                        );
                    }
                });
            });

            // Recovery suggestion (always visible, inline)
            if let Some(suggestion) = suggestion {
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(suggestion)
                            .color(secondary_color)
                            .italics()
                            .font(Typography::body_small()),
                    )
                    .wrap(),
                );
            }

            // Technical details (collapsible)
            if let Some(details) = details {
                ui.add_space(2.0);
                let toggle_text = if *details_expanded {
                    "Hide details"
                } else {
                    "Show details"
                };
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(toggle_text)
                                .font(Typography::body_small())
                                .color(DashColors::DASH_BLUE)
                                .underline(),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    *details_expanded = !*details_expanded;
                }

                if *details_expanded {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(DashColors::input_background(dark_mode))
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .corner_radius(Shape::RADIUS_SM as f32)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(DETAILS_MAX_HEIGHT)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(details)
                                                .font(Typography::monospace())
                                                .color(secondary_color),
                                        )
                                        .wrap(),
                                    );
                                });
                        });
                }
            }
        });
    ui.add_space(Spacing::SM);

    dismissed
}

/// Reads the global banner list from egui context data.
fn get_banners(ctx: &egui::Context) -> Vec<BannerState> {
    ctx.data(|d| d.get_temp::<Vec<BannerState>>(egui::Id::new(BANNER_STATE_ID)))
        .unwrap_or_default()
}

/// Writes the global banner list to egui context data.
/// Removes the entry entirely when the list is empty.
fn set_banners(ctx: &egui::Context, banners: Vec<BannerState>) {
    if banners.is_empty() {
        ctx.data_mut(|d| d.remove::<Vec<BannerState>>(egui::Id::new(BANNER_STATE_ID)));
    } else {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(BANNER_STATE_ID), banners));
    }
}

fn icon_for_type(message_type: MessageType) -> &'static str {
    match message_type {
        MessageType::Error => "\u{26D4}",   // no entry (⛔)
        MessageType::Warning => "\u{26A0}", // warning sign (⚠)
        MessageType::Success => "\u{2705}", // white heavy check mark (✅)
        MessageType::Info => "\u{1F4AC}",   // speech balloon (💬)
    }
}

// ---------------------------------------------------------------------------
// Extension traits for ergonomic banner display on Result and Option.
// ---------------------------------------------------------------------------

/// Extension for `Result<T, E>` — show an error banner on `Err`, pass through unchanged.
///
/// ```ignore
/// let wallet = get_selected_wallet(&identity, None, key)
///     .or_show_error(app_context.egui_ctx())
///     .unwrap_or(None);
/// ```
pub trait ResultBannerExt<T, E> {
    /// If `Err`, displays a global error banner with the error's `Display` text.
    /// Returns `self` unchanged — this is a side-effect-only method.
    ///
    /// INTENTIONAL(SEC-007): Raw `Display` text is shown directly. Callers must
    /// ensure error types have user-friendly Display implementations.
    fn or_show_error(self, ctx: &egui::Context) -> Self;
}

impl<T, E: fmt::Display> ResultBannerExt<T, E> for Result<T, E> {
    fn or_show_error(self, ctx: &egui::Context) -> Self {
        if let Err(ref e) = self {
            MessageBanner::set_global(ctx, e, MessageType::Error);
        }
        self
    }
}

/// Extension for `Option<T>` — show an error banner on `None`, pass through unchanged.
///
/// ```ignore
/// let identity = identities.first().cloned()
///     .or_show_error(ctx, "No identities loaded");
/// ```
pub trait OptionBannerShowExt<T> {
    /// If `None`, displays a global error banner with the given message.
    /// Returns `self` unchanged — this is a side-effect-only method.
    fn or_show_error(self, ctx: &egui::Context, msg: impl fmt::Display) -> Self;
}

impl<T> OptionBannerShowExt<T> for Option<T> {
    fn or_show_error(self, ctx: &egui::Context, msg: impl fmt::Display) -> Self {
        if self.is_none() {
            MessageBanner::set_global(ctx, msg, MessageType::Error);
        }
        self
    }
}

/// Extension for `Option<BannerHandle>` — banner lifecycle management.
///
/// Screens that run backend tasks typically store a `refresh_banner: Option<BannerHandle>`.
/// This trait provides convenience methods to clear and/or replace that banner atomically.
///
/// ```ignore
/// self.refresh_banner.take_and_clear();
/// self.refresh_banner.replace(ctx, "Loading...", MessageType::Info);
/// self.refresh_banner.replace_with_elapsed(ctx, "Refreshing...", MessageType::Info);
/// ```
pub trait OptionBannerExt {
    /// Takes the handle (leaving `None`) and clears the associated banner.
    fn take_and_clear(&mut self);

    /// Clears any existing banner, sets a new global banner, and stores the handle.
    fn replace(&mut self, ctx: &egui::Context, msg: impl fmt::Display, msg_type: MessageType);

    /// Like [`replace`](OptionBannerExt::replace), but also enables elapsed-time display on
    /// the new banner (useful for long-running operations).
    fn replace_with_elapsed(
        &mut self,
        ctx: &egui::Context,
        msg: impl fmt::Display,
        msg_type: MessageType,
    );
}

impl OptionBannerExt for Option<BannerHandle> {
    fn take_and_clear(&mut self) {
        if let Some(h) = self.take() {
            h.clear();
        }
    }

    fn replace(&mut self, ctx: &egui::Context, msg: impl fmt::Display, msg_type: MessageType) {
        self.take_and_clear();
        *self = Some(MessageBanner::set_global(ctx, msg.to_string(), msg_type));
    }

    fn replace_with_elapsed(
        &mut self,
        ctx: &egui::Context,
        msg: impl fmt::Display,
        msg_type: MessageType,
    ) {
        self.take_and_clear();
        let handle = MessageBanner::set_global(ctx, msg.to_string(), msg_type);
        handle.with_elapsed();
        *self = Some(handle);
    }
}
