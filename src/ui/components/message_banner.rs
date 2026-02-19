use crate::ui::MessageType;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{DashColors, Shape, Spacing, Typography};
use egui::InnerResponse;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_AUTO_DISMISS: Duration = Duration::from_secs(5);
const MAX_BANNERS: usize = 5;
const BANNER_STATE_ID: &str = "__global_message_banner";
/// Maximum height for the expanded details section before scrolling.
const DETAILS_MAX_HEIGHT: f32 = 120.0;

/// Monotonic counter for generating unique banner keys.
static BANNER_KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_banner_key() -> u64 {
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
}

/// Handle for a global banner, returned by [`MessageBanner::set_global`] and
/// [`MessageBanner::replace_global`]. Identifies the banner by an internal key,
/// so the display text can be updated without losing the reference.
///
/// The handle is `'static` and safe to store. Methods that modify the banner
/// (`set_text`, `with_auto_dismiss`) take `&self` so the handle can be reused.
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
    pub fn set_message(&self, text: &str) -> Option<&Self> {
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
    /// Returns `None` if the banner no longer exists.
    pub fn with_details(&self, details: &str) -> Option<&Self> {
        if details.is_empty() {
            return Some(self);
        }
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.details = Some(details.to_string());
        set_banners(&self.ctx, banners);
        Some(self)
    }

    /// Attach an optional recovery suggestion to this banner.
    /// The suggestion is shown inline (visible without expanding).
    /// Returns `None` if the banner no longer exists.
    pub fn with_suggestion(&self, suggestion: &str) -> Option<&Self> {
        if suggestion.is_empty() {
            return Some(self);
        }
        let mut banners = get_banners(&self.ctx);
        let b = banners.iter_mut().find(|b| b.key == self.key)?;
        b.suggestion = Some(suggestion.to_string());
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
    pub fn set_message(&mut self, text: &str, message_type: MessageType) -> &mut Self {
        if text.is_empty() {
            self.state = None;
        } else {
            self.state = Some(BannerState {
                key: next_banner_key(),
                text: text.to_string(),
                message_type,
                created_at: Instant::now(),
                auto_dismiss_after: default_auto_dismiss(message_type),
                show_elapsed: false,
                details: None,
                suggestion: None,
                details_expanded: false,
            });
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
    /// Evicts the oldest message when the cap ([`MAX_BANNERS`]) is reached.
    ///
    /// Returns a [`BannerHandle`] for updating or clearing the banner later.
    pub fn set_global(ctx: &egui::Context, text: &str, message_type: MessageType) -> BannerHandle {
        let mut banners = get_banners(ctx);
        if let Some(existing) = banners.iter().find(|b| b.text == text) {
            let key = existing.key;
            return BannerHandle {
                ctx: ctx.clone(),
                key,
            };
        }
        let key = next_banner_key();
        if !text.is_empty() {
            banners.push(BannerState {
                key,
                text: text.to_string(),
                message_type,
                created_at: Instant::now(),
                auto_dismiss_after: default_auto_dismiss(message_type),
                show_elapsed: false,
                details: None,
                suggestion: None,
                details_expanded: false,
            });
            if banners.len() > MAX_BANNERS {
                banners.remove(0);
            }
            set_banners(ctx, banners);
        }
        BannerHandle {
            ctx: ctx.clone(),
            key,
        }
    }

    /// Finds a message by `old_text` and replaces it with `new_text`.
    /// If `old_text` is not found, adds `new_text` as a new message (with dedup check).
    ///
    /// Returns a [`BannerHandle`] for updating or clearing the banner later.
    pub fn replace_global(
        ctx: &egui::Context,
        old_text: &str,
        new_text: &str,
        message_type: MessageType,
    ) -> BannerHandle {
        if new_text.is_empty() {
            Self::clear_global_message(ctx, old_text);
            return BannerHandle {
                ctx: ctx.clone(),
                key: next_banner_key(),
            };
        }
        let mut banners = get_banners(ctx);
        let key;
        if let Some(b) = banners.iter_mut().find(|b| b.text == old_text) {
            key = b.key;
            b.text = new_text.to_string();
            b.message_type = message_type;
            b.created_at = Instant::now();
            b.auto_dismiss_after = default_auto_dismiss(message_type);
            b.show_elapsed = false;
        } else if let Some(existing) = banners.iter().find(|b| b.text == new_text) {
            key = existing.key;
        } else {
            key = next_banner_key();
            banners.push(BannerState {
                key,
                text: new_text.to_string(),
                message_type,
                created_at: Instant::now(),
                auto_dismiss_after: default_auto_dismiss(message_type),
                show_elapsed: false,
                details: None,
                suggestion: None,
                details_expanded: false,
            });
            if banners.len() > MAX_BANNERS {
                banners.remove(0);
            }
        }
        set_banners(ctx, banners);
        BannerHandle {
            ctx: ctx.clone(),
            key,
        }
    }

    /// Clears the specific global banner message matching `text`.
    pub fn clear_global_message(ctx: &egui::Context, text: &str) {
        let mut banners = get_banners(ctx);
        banners.retain(|b| b.text != text);
        set_banners(ctx, banners);
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
            ui.horizontal(|ui| {
                // Icon
                ui.label(egui::RichText::new(icon).color(fg_color).strong());
                ui.add_space(Spacing::XS);

                // Message text
                ui.label(egui::RichText::new(text).color(fg_color));

                // Right-aligned: annotation + dismiss
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").clicked() {
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
        MessageType::Error => "\u{26A0}",   // warning sign
        MessageType::Warning => "\u{26A0}", // warning sign (differentiated by color)
        MessageType::Success => "\u{2713}", // check mark
        MessageType::Info => "\u{2139}",    // info
    }
}
