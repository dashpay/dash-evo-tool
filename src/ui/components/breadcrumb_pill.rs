//! Breadcrumb pill — label + optional icon + optional chevron rendered in the
//! topbar breadcrumb row of the Identities hub.
//!
//! Three visual modes:
//!
//! - `Interactive` — hover border, chevron, clickable. Used for wallet /
//!   identity pills that open a dropdown.
//! - `Subdued`     — no chevron, no hover treatment, transparent background.
//!   Used when the segment is informational only (Alex's one-wallet case).
//! - `Placeholder` — italic, text-secondary, non-interactive. Used when the
//!   segment has no value yet: `(no wallet yet)`, `(no identity yet)`, etc.
//!
//! See design-spec §A.3.
//!
//! This component follows `docs/COMPONENT_DESIGN_PATTERN.md`:
//! - private fields + builder methods for configuration,
//! - a response struct carrying interaction state (not the widget),
//! - self-contained theming for light + dark mode.

use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::theme::{DashColors, ResponseExt};
use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, WidgetInfo, WidgetType,
};

/// The three rendering modes for a breadcrumb pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreadcrumbPillMode {
    /// Fully interactive. Hover highlights, chevron visible, clickable.
    #[default]
    Interactive,
    /// Non-interactive, subdued surface, no chevron. For single-option segments.
    Subdued,
    /// Placeholder text (italic, text-secondary). Non-interactive.
    Placeholder,
}

/// Response struct returned by [`BreadcrumbPill::show`].
///
/// Carries whether the user clicked the pill. `has_changed` returns `true`
/// exclusively on click — this is the "state change" callers care about. When
/// clicked, `changed_value` yields the pill's label so a switcher composition
/// can report which segment was activated.
#[derive(Clone, Debug)]
pub struct BreadcrumbPillResponse {
    pub clicked: bool,
    pub label: String,
    pub mode: BreadcrumbPillMode,
    changed_value: Option<String>,
}

impl BreadcrumbPillResponse {
    /// Construct a response directly. Exposed at crate visibility for
    /// compositional callers (e.g. the identity pill wrapper) and for
    /// integration tests that fabricate responses without running egui.
    pub(crate) fn new(label: String, mode: BreadcrumbPillMode, clicked: bool) -> Self {
        let changed_value = if clicked { Some(label.clone()) } else { None };
        Self {
            clicked,
            label,
            mode,
            changed_value,
        }
    }
}

impl ComponentResponse for BreadcrumbPillResponse {
    type DomainType = String;

    fn has_changed(&self) -> bool {
        self.clicked
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.changed_value
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// A breadcrumb pill widget.
#[derive(Clone, Debug)]
pub struct BreadcrumbPill {
    label: String,
    icon: Option<String>,
    mode: BreadcrumbPillMode,
    /// Tooltip text. Empty string = no tooltip.
    tooltip: String,
    /// Accessible name override. Defaults to the visible label.
    accessible_name: Option<String>,
}

impl BreadcrumbPill {
    /// Build an interactive pill with the given visible label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            mode: BreadcrumbPillMode::default(),
            tooltip: String::new(),
            accessible_name: None,
        }
    }

    /// Build a placeholder pill (italic, text-secondary, non-interactive).
    pub fn placeholder(label: impl Into<String>) -> Self {
        let mut pill = Self::new(label);
        pill.mode = BreadcrumbPillMode::Placeholder;
        pill
    }

    /// Flip subdued mode on or off. Subdued pills have no chevron and no hover
    /// treatment.
    pub fn subdued(mut self, subdued: bool) -> Self {
        if subdued {
            self.mode = BreadcrumbPillMode::Subdued;
        } else if matches!(self.mode, BreadcrumbPillMode::Subdued) {
            self.mode = BreadcrumbPillMode::Interactive;
        }
        self
    }

    /// Attach an icon glyph shown before the label. Emoji-safe short strings.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Attach a tooltip. Empty string disables the tooltip.
    pub fn with_tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = text.into();
        self
    }

    /// Override the accessible name. Defaults to the visible label.
    pub fn with_accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible_name = Some(name.into());
        self
    }

    /// Force a specific mode. Useful for tests and compositional callers that
    /// need to bypass [`new`] / [`placeholder`].
    pub fn with_mode(mut self, mode: BreadcrumbPillMode) -> Self {
        self.mode = mode;
        self
    }

    /// The pill's visible label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The pill's current mode.
    pub fn mode(&self) -> BreadcrumbPillMode {
        self.mode
    }

    /// Whether the pill renders in subdued mode.
    pub fn is_subdued(&self) -> bool {
        matches!(self.mode, BreadcrumbPillMode::Subdued)
    }

    /// Whether the pill is a placeholder.
    pub fn is_placeholder(&self) -> bool {
        matches!(self.mode, BreadcrumbPillMode::Placeholder)
    }

    /// Whether the pill is interactive (handles clicks).
    pub fn is_interactive(&self) -> bool {
        matches!(self.mode, BreadcrumbPillMode::Interactive)
    }

    /// Render the pill and return a response.
    ///
    /// Only [`BreadcrumbPillMode::Interactive`] pills report `clicked == true`;
    /// the other modes are non-interactive by design and their response always
    /// has `clicked == false`.
    pub fn show(self, ui: &mut Ui) -> BreadcrumbPillResponse {
        let dark_mode = ui.ctx().global_style().visuals.dark_mode;
        let accessible_name = self
            .accessible_name
            .clone()
            .unwrap_or_else(|| self.label.clone());

        let (text_color, bg, stroke) = match self.mode {
            BreadcrumbPillMode::Interactive => (
                DashColors::text_primary(dark_mode),
                DashColors::surface(dark_mode),
                Stroke::new(1.0, DashColors::border_light(dark_mode)),
            ),
            BreadcrumbPillMode::Subdued => (
                DashColors::text_secondary(dark_mode),
                Color32::TRANSPARENT,
                Stroke::NONE,
            ),
            BreadcrumbPillMode::Placeholder => (
                DashColors::text_secondary(dark_mode),
                Color32::TRANSPARENT,
                Stroke::NONE,
            ),
        };

        // Build the label text with optional icon prefix + chevron suffix.
        let mut display = String::new();
        if let Some(icon) = &self.icon {
            display.push_str(icon);
            display.push(' ');
        }
        display.push_str(&self.label);
        if matches!(self.mode, BreadcrumbPillMode::Interactive) {
            display.push_str(" ▾");
        }

        let mut rich = RichText::new(display).color(text_color);
        if matches!(self.mode, BreadcrumbPillMode::Placeholder) {
            rich = rich.italics();
        }

        let frame = Frame::new()
            .fill(bg)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(255))
            .inner_margin(Margin::symmetric(8, 2));
        let sense = if matches!(self.mode, BreadcrumbPillMode::Interactive) {
            Sense::click()
        } else {
            Sense::hover()
        };

        // In egui, `Frame::show(...).response` is the outer allocation, which
        // only senses `Sense::hover` by default — child-widget clicks are NOT
        // inherited by the outer Response. We must read `.inner` (the value
        // returned from the closure) to pick up the Label's click Sense.
        // Using the frame's `.response` instead silently breaks click
        // detection on interactive pills. See CodeRabbit review on PR #842 for
        // the full analysis.
        let inner_response = frame
            .show(ui, |ui| ui.add(egui::Label::new(rich).sense(sense)))
            .inner;

        let response = if !self.tooltip.is_empty() {
            if matches!(self.mode, BreadcrumbPillMode::Interactive) {
                inner_response.clickable_tooltip(&self.tooltip)
            } else {
                inner_response.info_tooltip(&self.tooltip)
            }
        } else {
            inner_response
        };
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Link,
                matches!(self.mode, BreadcrumbPillMode::Interactive),
                accessible_name.clone(),
            )
        });

        let clicked = matches!(self.mode, BreadcrumbPillMode::Interactive) && response.clicked();
        BreadcrumbPillResponse::new(self.label.clone(), self.mode, clicked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_label() {
        let pill = BreadcrumbPill::new("Main Wallet");
        assert_eq!(pill.label(), "Main Wallet");
        assert!(pill.is_interactive());
        assert!(!pill.is_subdued());
        assert!(!pill.is_placeholder());
    }

    #[test]
    fn subdued_flag_toggles_mode() {
        let pill = BreadcrumbPill::new("Main Wallet").subdued(true);
        assert!(pill.is_subdued());
        assert!(!pill.is_interactive());
        let pill = pill.subdued(false);
        assert!(pill.is_interactive());
    }

    #[test]
    fn placeholder_ctor_sets_mode() {
        let pill = BreadcrumbPill::placeholder("(no wallet yet)");
        assert_eq!(pill.label(), "(no wallet yet)");
        assert!(pill.is_placeholder());
        assert!(!pill.is_interactive());
    }

    #[test]
    fn response_roundtrip_interactive_not_clicked() {
        let resp = BreadcrumbPillResponse::new(
            "Main Wallet".to_string(),
            BreadcrumbPillMode::Interactive,
            false,
        );
        assert!(!resp.has_changed());
        assert!(resp.is_valid());
        assert!(resp.changed_value().is_none());
        assert_eq!(resp.label, "Main Wallet");
    }

    #[test]
    fn response_roundtrip_interactive_clicked() {
        let resp = BreadcrumbPillResponse::new(
            "Main Wallet".to_string(),
            BreadcrumbPillMode::Interactive,
            true,
        );
        assert!(resp.has_changed());
        assert_eq!(resp.changed_value().as_deref(), Some("Main Wallet"));
    }

    #[test]
    fn placeholder_response_never_clicked() {
        let resp = BreadcrumbPillResponse::new(
            "(no wallet yet)".to_string(),
            BreadcrumbPillMode::Placeholder,
            false,
        );
        assert!(!resp.has_changed());
        assert_eq!(resp.mode, BreadcrumbPillMode::Placeholder);
    }

    #[test]
    fn builder_chain_is_fluent() {
        let pill = BreadcrumbPill::new("alex.dash")
            .with_icon("👤")
            .with_tooltip("Switch between identities")
            .with_accessible_name("Identity switcher");
        assert_eq!(pill.label(), "alex.dash");
        assert_eq!(pill.mode(), BreadcrumbPillMode::Interactive);
    }
}
