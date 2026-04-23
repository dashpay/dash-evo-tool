//! Horizontal tab bar for the unified Identities hub.
//!
//! Renders the four top-level tabs — `Home`, `Contacts`, `Activity`, `Settings`
//! — as a single-row, selectable button strip that sits directly beneath the
//! breadcrumb row on every hub tab. The selected tab uses the project's
//! `DashColors::DASH_BLUE` accent fill; unselected tabs use the transparent /
//! border-light treatment shared with the existing subscreen chooser panels.
//!
//! This component is **not** a replacement for the per-section chooser panels
//! under `src/ui/components/*_subscreen_chooser_panel.rs` — those continue to
//! live on the legacy screens. This tab bar is specific to the hub and is
//! consumed only from `src/ui/identity/hub_screen.rs`.
//!
//! See:
//!
//! - `docs/ai-design/2026-04-22-identity-dashpay-redesign/design-spec.md` §A.2
//!   (tab catalog) and the `Identity Home (Frame 3)` wireframe for the chrome
//!   strip placement.
//! - `docs/ai-design/2026-04-23-identity-hub-impl/03-test-case-spec.md`
//!   UT-TABS-01.
//! - `docs/COMPONENT_DESIGN_PATTERN.md` for the component pattern applied
//!   here (private fields, builder methods, `ComponentResponse` trait).

use crate::ui::components::component_trait::ComponentResponse;
use crate::ui::identity::IdentityHubTab;
use crate::ui::theme::{DashColors, ResponseExt, Shape, Spacing, Typography};
use eframe::egui::{self, CornerRadius, RichText, Stroke, Ui, Vec2};

/// Response returned by [`IdentityHubTabBar::show`].
///
/// Carries the tab (if any) that was activated on this frame. The component is
/// fully controlled by the parent — internal selection state lives in the
/// screen, the bar simply reports click events.
#[derive(Clone, Debug, Default)]
pub struct IdentityHubTabBarResponse {
    clicked: Option<IdentityHubTab>,
}

impl IdentityHubTabBarResponse {
    /// The tab the user activated on this frame, if any.
    pub fn clicked(&self) -> Option<IdentityHubTab> {
        self.clicked
    }
}

impl ComponentResponse for IdentityHubTabBarResponse {
    type DomainType = IdentityHubTab;

    fn has_changed(&self) -> bool {
        self.clicked.is_some()
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        &self.clicked
    }

    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// Horizontal tab bar for the Identities hub.
///
/// Construct with the currently selected tab; render with [`show`]; apply the
/// returned click (if any) back to the screen's own selection state. The bar
/// is stateless — each frame receives the current selection as input and
/// reports a click as output.
///
/// [`show`]: IdentityHubTabBar::show
#[derive(Clone, Debug)]
pub struct IdentityHubTabBar {
    selected: IdentityHubTab,
    /// Optional min button width override. Defaults to `96.0`, which
    /// comfortably fits every tab label at `Typography::SCALE_SM`.
    min_button_width: f32,
}

impl IdentityHubTabBar {
    /// Build a new tab bar with the given tab pre-selected.
    pub fn new(selected: IdentityHubTab) -> Self {
        Self {
            selected,
            min_button_width: 96.0,
        }
    }

    /// Override the minimum per-tab button width. Useful in narrow layouts.
    pub fn with_min_button_width(mut self, width: f32) -> Self {
        self.min_button_width = width;
        self
    }

    /// The currently selected tab.
    pub fn selected(&self) -> IdentityHubTab {
        self.selected
    }

    /// Render the bar and return a response describing any click. The caller
    /// is responsible for applying `response.clicked()` to its own selection
    /// state — this component never mutates the selection itself.
    pub fn show(self, ui: &mut Ui) -> IdentityHubTabBarResponse {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut response = IdentityHubTabBarResponse::default();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = Spacing::SM;
            for tab in IdentityHubTab::ALL {
                let is_selected = tab == self.selected;
                let label = tab.label();

                let button = if is_selected {
                    egui::Button::new(
                        RichText::new(label)
                            .color(DashColors::WHITE)
                            .size(Typography::SCALE_SM)
                            .strong(),
                    )
                    .fill(DashColors::DASH_BLUE)
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
                    .min_size(Vec2::new(self.min_button_width, 32.0))
                } else {
                    egui::Button::new(
                        RichText::new(label)
                            .color(DashColors::text_primary(dark_mode))
                            .size(Typography::SCALE_SM),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(Stroke::new(1.0, DashColors::border_light(dark_mode)))
                    .corner_radius(CornerRadius::same(Shape::RADIUS_MD))
                    .min_size(Vec2::new(self.min_button_width, 32.0))
                };

                let clicked = ui
                    .add(button)
                    .clickable_tooltip(tab.accessible_description())
                    .clicked();
                if clicked && !is_selected {
                    response.clicked = Some(tab);
                }
            }
        });

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_selected_tab() {
        let bar = IdentityHubTabBar::new(IdentityHubTab::Contacts);
        assert_eq!(bar.selected(), IdentityHubTab::Contacts);
    }

    #[test]
    fn default_response_has_no_click() {
        let response = IdentityHubTabBarResponse::default();
        assert!(!response.has_changed());
        assert!(response.is_valid());
        assert!(response.changed_value().is_none());
        assert!(response.error_message().is_none());
        assert!(response.clicked().is_none());
    }

    #[test]
    fn response_reports_click_value() {
        let response = IdentityHubTabBarResponse {
            clicked: Some(IdentityHubTab::Contacts),
        };
        assert!(response.has_changed());
        assert_eq!(response.clicked(), Some(IdentityHubTab::Contacts));
        assert_eq!(
            response.changed_value().as_ref(),
            Some(&IdentityHubTab::Contacts)
        );
    }

    #[test]
    fn min_button_width_is_overridable() {
        let bar = IdentityHubTabBar::new(IdentityHubTab::Home).with_min_button_width(128.0);
        assert_eq!(bar.min_button_width, 128.0);
    }

    // UT-TABS-01 — `IdentityHubTabBar` selection.
    //
    // Preconditions: tab bar with all four tabs, selected = Home.
    // Steps: simulate a Contacts-tab click via the component's response API.
    // Expected: response reports `Some(IdentityHubTab::Contacts)`; the
    // screen-side selection updates accordingly.
    //
    // Rendering + raw click dispatch is covered end-to-end by the kittest
    // integration test (`tests/kittest/identity_hub_onboarding.rs` and follow-
    // up home-tab tests). This unit test exercises the pure state contract
    // between the bar and its consumer, which is what the component owns.
    #[test]
    fn ut_tabs_01_selection_round_trips_through_response() {
        let mut selected = IdentityHubTab::Home;

        // Initial render: nothing clicked.
        let idle = IdentityHubTabBarResponse::default();
        assert_eq!(idle.clicked(), None);
        assert!(!idle.has_changed());

        // Simulate the component reporting a click on Contacts.
        let click = IdentityHubTabBarResponse {
            clicked: Some(IdentityHubTab::Contacts),
        };
        if let Some(tab) = click.clicked() {
            selected = tab;
        }
        assert_eq!(selected, IdentityHubTab::Contacts);
        assert_eq!(
            click.changed_value().as_ref(),
            Some(&IdentityHubTab::Contacts)
        );

        // Construct a new bar with the updated selection — bar state tracks
        // the screen's source of truth.
        let bar = IdentityHubTabBar::new(selected);
        assert_eq!(bar.selected(), IdentityHubTab::Contacts);
    }
}
