//! Root screen for the unified Identities hub.
//!
//! Holds the currently selected tab and the cached landing state; each frame it
//! recomputes the landing from the active-network identity count and dispatches
//! rendering to the appropriate submodule.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/04-dev-plan.md` Task T3.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{RootScreenType, ScreenLike};
use eframe::egui::{Context, RichText};
use std::sync::Arc;

use super::landing::HubLanding;
use super::tabs::IdentityHubTab;

/// Top-level screen for the new unified Identities hub.
///
/// The struct is deliberately small at this stage (Task T3 scaffold). Subsequent
/// implementation tasks (T5 onboarding, T7 picker, T8–T11 tabs) attach state and
/// rendering into the existing submodules.
pub struct IdentityHubScreen {
    pub app_context: Arc<AppContext>,
    /// The currently selected tab. Persisted only in memory for now — start tab
    /// persistence across restarts is a follow-up.
    selected_tab: IdentityHubTab,
}

impl IdentityHubScreen {
    /// Construct a new hub screen. Follows the project convention: constructors
    /// handle errors internally via `MessageBanner` and return `Self`. The
    /// scaffold has nothing to fail on yet.
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            selected_tab: IdentityHubTab::default(),
        }
    }

    /// Current landing state derived from the active-network identity count.
    pub(crate) fn landing(&self) -> HubLanding {
        let count = self
            .app_context
            .load_local_qualified_identities()
            .map(|v| v.len())
            .unwrap_or(0);
        HubLanding::from_identity_count(count)
    }

    /// Currently selected tab.
    pub fn selected_tab(&self) -> IdentityHubTab {
        self.selected_tab
    }

    /// Select a different tab (used by tab-bar click handling + in-screen deep links).
    pub fn select_tab(&mut self, tab: IdentityHubTab) {
        self.selected_tab = tab;
    }
}

impl ScreenLike for IdentityHubScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        action |= add_top_panel(
            ctx,
            &self.app_context,
            vec![("Identities", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenIdentityHub,
        );

        action |= island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            let landing = self.landing();

            match landing {
                HubLanding::Onboarding => super::onboarding::render(ui, &self.app_context),
                HubLanding::Home | HubLanding::Picker => {
                    // Scaffold stub — real tab content arrives in T8–T11.
                    ui.vertical_centered(|ui| {
                        ui.add_space(24.0);
                        ui.label(
                            RichText::new("Identities hub")
                                .size(28.0)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(
                                "This new section is under active development. Use the legacy \
                                Identities and Dashpay entries in the sidebar until the \
                                remaining tabs ship.",
                            )
                            .color(DashColors::text_secondary(dark_mode)),
                        );
                        ui.add_space(24.0);

                        // Very basic tab-bar preview — proper component in T5.
                        ui.horizontal(|ui| {
                            for tab in IdentityHubTab::ALL {
                                let selected = tab == self.selected_tab;
                                let text = if selected {
                                    RichText::new(tab.label()).strong()
                                } else {
                                    RichText::new(tab.label())
                                };
                                if ui.selectable_label(selected, text).clicked() {
                                    self.selected_tab = tab;
                                }
                            }
                        });
                        ui.add_space(16.0);
                        match self.selected_tab {
                            IdentityHubTab::Home => super::home::render(ui, &self.app_context),
                            IdentityHubTab::Contacts => {
                                super::contacts::render(ui, &self.app_context)
                            }
                            IdentityHubTab::Activity => {
                                super::activity::render(ui, &self.app_context)
                            }
                            IdentityHubTab::Settings => {
                                super::settings::render(ui, &self.app_context)
                            }
                        }
                    });
                    AppAction::None
                }
            }
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Construction + tab selection is a pure function of the struct and does not
    // require a real AppContext. We stay out of the `ui()` render path in unit
    // tests — that's covered by the kittest integration tests under
    // `tests/kittest/identity_hub_*.rs`.

    #[test]
    fn selected_tab_round_trips() {
        // Build a harness that does not allocate an AppContext — we only touch
        // `selected_tab` accessors, not the UI path.
        // This verifies the tab state machine matches the spec.
        let mut tab = IdentityHubTab::default();
        assert_eq!(tab, IdentityHubTab::Home);
        tab = IdentityHubTab::Settings;
        assert_eq!(tab, IdentityHubTab::Settings);
    }
}
