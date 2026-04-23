//! Activity row — a compact 48 px timeline row used by the Activity tab of the
//! Identities hub.
//!
//! Design reference: `docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md` §B.6
//! (Activity tab / Frame 5) and `wireframe.html` Frame F6.
//!
//! # Variants
//!
//! Each row has two orthogonal dimensions:
//!
//! - [`ActivityRowKind`] — whether this row represents a payment, a funding
//!   operation, or a platform op. Drives the left-edge icon badge color.
//! - [`ActivityRowStatus`] — `Normal`, `Expanded`, or `Failed`. Only `Failed`
//!   rows render the `Retry` affordance and a red left-border accent.
//!
//! # Interaction
//!
//! The returned [`ActivityRowResponse`] reports two possible actions via
//! [`ActivityRowAction`]:
//!
//! - `ToggleExpand` — the user clicked the row body or expand chevron.
//! - `Retry`        — the user clicked the `Retry` small button on a failed row.
//!
//! This component follows `docs/COMPONENT_DESIGN_PATTERN.md`: private fields,
//! builder methods, self-contained theming for light + dark mode.

use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{DashColors, Shape};
use eframe::egui::{
    self, Color32, CornerRadius, Frame, InnerResponse, Margin, RichText, Sense, Stroke, Ui, Vec2,
};

/// The category of activity a row represents.
///
/// Drives the color of the left-edge icon badge and is used by the Activity tab
/// filter chips. Kept as a pure enum so unit tests can exhaustively cover
/// rendering paths without constructing a full activity payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityRowKind {
    /// Money sent or received via DashPay.
    Payment,
    /// Funding operation: add funds, send to wallet, send to another identity.
    Funding,
    /// Platform operation: DPNS registration, key change, contract interaction.
    PlatformOp,
}

/// Visual status of the row.
///
/// `Failed` is a distinct variant (not a flag) because the Failed row is
/// functionally and visually different — it renders a retry button, a red
/// left-border accent, and a calm error sub-copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityRowStatus {
    /// Normal collapsed row. Expandable via click.
    Normal,
    /// Row currently expanded; shows detail sub-panel.
    Expanded,
    /// Payment or funding operation that failed; shows Retry.
    Failed,
}

/// The action a user took on an activity row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityRowAction {
    /// The user clicked the row body / expand chevron — caller should toggle
    /// the expanded state of the row in its own state.
    ToggleExpand,
    /// The user clicked the `Retry` button on a `Failed` row.
    Retry,
}

/// Response returned by [`ActivityRow::show`].
#[derive(Clone, Debug)]
pub struct ActivityRowResponse {
    /// Whether the user clicked the row body or the expand chevron.
    pub clicked_body: bool,
    /// Whether the user clicked the `Retry` button (only ever `true` for
    /// `Failed` rows).
    pub clicked_retry: bool,
    /// The resolved action, if any.
    action: Option<ActivityRowAction>,
}

impl ActivityRowResponse {
    fn new(clicked_body: bool, clicked_retry: bool) -> Self {
        // Retry takes precedence over body click because the retry button sits
        // on top of the row surface and egui will not surface both on the same
        // frame, but the explicit ordering keeps the response deterministic.
        let action = if clicked_retry {
            Some(ActivityRowAction::Retry)
        } else if clicked_body {
            Some(ActivityRowAction::ToggleExpand)
        } else {
            None
        };
        Self {
            clicked_body,
            clicked_retry,
            action,
        }
    }

    /// The resolved action, if any.
    pub fn action(&self) -> Option<ActivityRowAction> {
        self.action
    }
}

impl ComponentResponse for ActivityRowResponse {
    type DomainType = ActivityRowAction;

    fn has_changed(&self) -> bool {
        self.action.is_some()
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

/// A 48 px compact row used by the unified Activity timeline.
#[derive(Clone, Debug)]
pub struct ActivityRow {
    kind: ActivityRowKind,
    status: ActivityRowStatus,
    title: String,
    subtitle: String,
    timestamp: String,
    /// Optional detail text shown when the row is in `Expanded` or `Failed`
    /// status. For `Failed`, the design-spec recommends the copy:
    /// "The network did not accept this payment. Your balance is unchanged.
    /// Check your connection and try again, or try a smaller amount."
    detail: Option<String>,
}

impl ActivityRow {
    /// Build a new activity row of the given kind with a normal status.
    pub fn new(kind: ActivityRowKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            status: ActivityRowStatus::Normal,
            title: title.into(),
            subtitle: String::new(),
            timestamp: String::new(),
            detail: None,
        }
    }

    /// Set the row's visual status.
    pub fn with_status(mut self, status: ActivityRowStatus) -> Self {
        self.status = status;
        self
    }

    /// Attach a subtitle (counterparty + method line).
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    /// Attach a timestamp (human-readable, pre-formatted by the caller).
    pub fn with_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.timestamp = timestamp.into();
        self
    }

    /// Attach a detail body shown in `Expanded` or `Failed` status.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The row's kind.
    pub fn kind(&self) -> ActivityRowKind {
        self.kind
    }

    /// The row's current status.
    pub fn status(&self) -> ActivityRowStatus {
        self.status
    }

    /// Whether the row currently renders a `Retry` button (i.e. is failed).
    pub fn has_retry(&self) -> bool {
        matches!(self.status, ActivityRowStatus::Failed)
    }

    /// The stroke color for the left-edge accent.
    ///
    /// Kept as a pure accessor — we intentionally pick the same color in
    /// light and dark mode for now because the palette constants already
    /// carry enough contrast. `dark_mode` is plumbed through for future
    /// tinting without breaking callers.
    fn accent_color(&self, _dark_mode: bool) -> Color32 {
        match (self.status, self.kind) {
            (ActivityRowStatus::Failed, _) => DashColors::ERROR,
            (_, ActivityRowKind::Payment) => DashColors::DASH_BLUE,
            (_, ActivityRowKind::Funding) => DashColors::SUCCESS,
            (_, ActivityRowKind::PlatformOp) => DashColors::INFO,
        }
    }
}

impl Component for ActivityRow {
    type DomainType = ActivityRowAction;
    type Response = ActivityRowResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        let accent = self.accent_color(dark_mode);
        let surface = DashColors::surface(dark_mode);

        // Failed rows carry a distinct danger-tinted border; normal rows keep
        // a subtle card stroke.
        let stroke = if matches!(self.status, ActivityRowStatus::Failed) {
            Stroke::new(1.0, DashColors::ERROR)
        } else {
            Stroke::new(1.0, DashColors::border_light(dark_mode))
        };

        let frame = Frame {
            fill: surface,
            stroke,
            corner_radius: CornerRadius::same(Shape::RADIUS_SM),
            inner_margin: Margin::symmetric(12, 8),
            outer_margin: Margin::ZERO,
            shadow: egui::epaint::Shadow::NONE,
        };

        let mut clicked_body = false;
        let mut clicked_retry = false;

        let response = frame.show(ui, |ui| {
            // Fixed-height row = 48 px minus vertical inner margin (8 + 8 = 16)
            // so the content band is 32 px tall, matching the design-spec row.
            ui.set_min_height(32.0);
            ui.horizontal(|ui| {
                // Left accent badge: small colored square standing in for the
                // icon slot. Kept as a painted rect (not an icon font) so the
                // component has no external asset dependency.
                let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 24.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(Shape::RADIUS_SM), accent);
                ui.add_space(8.0);

                // Center body: title + subtitle stacked.
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&self.title)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    if !self.subtitle.is_empty() {
                        ui.label(
                            RichText::new(&self.subtitle)
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                });

                // Right cluster: timestamp + expand chevron + optional Retry.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Retry button — only on Failed rows. Must be the
                    // right-most affordance so it does not conflict with
                    // the body click target.
                    if matches!(self.status, ActivityRowStatus::Failed) {
                        if ui.small_button(RichText::new("Retry").strong()).clicked() {
                            clicked_retry = true;
                        }
                        ui.add_space(8.0);
                    }

                    // Chevron: indicates expandability. Clickable surface
                    // is the whole row, but we render the chevron glyph
                    // here so the affordance is visually discoverable.
                    let chevron = match self.status {
                        ActivityRowStatus::Expanded => "▴",
                        _ => "▾",
                    };
                    ui.label(RichText::new(chevron).color(DashColors::text_secondary(dark_mode)));

                    if !self.timestamp.is_empty() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(&self.timestamp)
                                .small()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                });
            });

            // Expanded or Failed rows show a detail body below the main row.
            if matches!(
                self.status,
                ActivityRowStatus::Expanded | ActivityRowStatus::Failed
            ) && let Some(detail) = &self.detail
            {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(detail)
                        .small()
                        .color(DashColors::text_secondary(dark_mode)),
                );
            }
        });

        // Make the whole frame click-sensitive so users can click anywhere on
        // the row to toggle expansion. The retry button consumes its own
        // click first, so we only treat the body click as a toggle when the
        // retry button did not fire.
        let body_response = response.response.interact(Sense::click());
        if body_response.clicked() && !clicked_retry {
            clicked_body = true;
        }

        InnerResponse::new(
            ActivityRowResponse::new(clicked_body, clicked_retry),
            body_response,
        )
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        // Rows are stateless from the component's perspective — state lives on
        // the caller's list. There is no "current value" to report until the
        // user interacts with the row.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// Constructor smoke: defaults are sensible.
    #[test]
    fn new_defaults_to_normal_status() {
        let row = ActivityRow::new(ActivityRowKind::Payment, "Sent 1 DASH");
        assert_eq!(row.kind(), ActivityRowKind::Payment);
        assert_eq!(row.status(), ActivityRowStatus::Normal);
        assert!(!row.has_retry());
    }

    /// Builder methods compose.
    #[test]
    fn builder_methods_chain() {
        let row = ActivityRow::new(ActivityRowKind::Funding, "Added funds")
            .with_subtitle("From wallet · 2 min ago")
            .with_timestamp("2 min ago")
            .with_status(ActivityRowStatus::Expanded)
            .with_detail("Advanced details...");
        assert_eq!(row.status(), ActivityRowStatus::Expanded);
        assert!(!row.has_retry());
    }

    /// Failed rows report `has_retry() == true`.
    #[test]
    fn failed_row_has_retry() {
        let row = ActivityRow::new(ActivityRowKind::Payment, "Could not send 1 DASH")
            .with_status(ActivityRowStatus::Failed);
        assert!(row.has_retry());
    }

    /// Response: no action until the user clicks.
    #[test]
    fn response_is_empty_by_default() {
        let response = ActivityRowResponse::new(false, false);
        assert!(!response.has_changed());
        assert_eq!(response.action(), None);
        assert!(response.changed_value().is_none());
    }

    /// Response: body click yields `ToggleExpand`.
    #[test]
    fn body_click_yields_toggle_expand() {
        let response = ActivityRowResponse::new(true, false);
        assert!(response.has_changed());
        assert_eq!(response.action(), Some(ActivityRowAction::ToggleExpand));
    }

    /// Response: retry click yields `Retry` and wins over a stale body click.
    #[test]
    fn retry_click_yields_retry_and_takes_precedence() {
        let response = ActivityRowResponse::new(true, true);
        assert_eq!(response.action(), Some(ActivityRowAction::Retry));
    }

    /// UT-ACTIVITY-ROW-01 — Failed row renders a Retry button and a
    /// danger-stroke border.
    ///
    /// The test covers three variants in a single harness run so we
    /// exercise the Normal, Expanded, and Failed render paths together,
    /// as called out in the test-case spec.
    #[test]
    fn ut_activity_row_01_failed_row_has_retry_button() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(480.0, 400.0))
            .build_ui(|ui| {
                let mut normal = ActivityRow::new(ActivityRowKind::Payment, "Sent 0.1 DASH")
                    .with_subtitle("To @alice")
                    .with_timestamp("5 min ago");
                let normal_response = normal.show(ui);
                assert!(normal_response.inner.action().is_none());

                let mut expanded = ActivityRow::new(ActivityRowKind::Funding, "Added funds")
                    .with_subtitle("From wallet")
                    .with_timestamp("1 h ago")
                    .with_status(ActivityRowStatus::Expanded)
                    .with_detail("Advanced details.");
                let expanded_response = expanded.show(ui);
                assert!(expanded_response.inner.action().is_none());

                let mut failed =
                    ActivityRow::new(ActivityRowKind::Payment, "Could not send 0.1 DASH to @bob")
                        .with_timestamp("just now")
                        .with_status(ActivityRowStatus::Failed)
                        .with_detail(
                            "The network did not accept this payment. \
                             Your balance is unchanged. Check your connection \
                             and try again, or try a smaller amount.",
                        );
                let failed_response = failed.show(ui);
                assert!(failed.has_retry());
                // No interaction simulated, so no action yet.
                assert!(failed_response.inner.action().is_none());
            });
        harness.run();

        // The Retry button must be present — it is the distinguishing
        // affordance of the Failed variant.
        assert!(
            harness.query_by_label("Retry").is_some(),
            "Failed row must render a Retry button"
        );
        // Normal and Expanded titles must render.
        assert!(harness.query_by_label("Sent 0.1 DASH").is_some());
        assert!(harness.query_by_label("Added funds").is_some());
        assert!(
            harness
                .query_by_label("Could not send 0.1 DASH to @bob")
                .is_some()
        );
        // The expanded detail text must be visible.
        assert!(harness.query_by_label("Advanced details.").is_some());
    }
}
