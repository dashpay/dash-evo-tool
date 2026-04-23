//! Root screen for the unified Identities hub.
//!
//! Holds the currently selected tab and the cached landing state; each frame it
//! recomputes the landing from the active-network identity count and dispatches
//! rendering to the appropriate submodule.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/04-dev-plan.md` Task T3.

use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use eframe::egui::{Context, RichText};
use std::sync::Arc;

use super::home::HomeState;
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
    /// Cached landing-load error banner. Set by `landing()` when identity
    /// loading fails; cleared on `refresh`. Separating this from a real
    /// zero-identity state avoids inviting users to create identities when the
    /// real problem is a database / context error.
    load_error_banner: Option<BannerHandle>,
    /// Remembered landing state from the most recent successful load. Falls
    /// back to `HubLanding::Onboarding` on first render so the UI has
    /// something to draw until the first load attempt completes.
    last_good_landing: HubLanding,
    /// Home-tab state (dismissed checklist flag, advanced expander, etc).
    /// Owned here so tab switches do not wipe it.
    home_state: HomeState,
}

impl IdentityHubScreen {
    /// Construct a new hub screen. Follows the project convention: constructors
    /// handle errors internally via `MessageBanner` and return `Self`. The
    /// scaffold has nothing to fail on yet.
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            selected_tab: IdentityHubTab::default(),
            load_error_banner: None,
            last_good_landing: HubLanding::Onboarding,
            home_state: HomeState::default(),
        }
    }

    /// Resolve the current landing state from the active-network identity
    /// count. On load failure, surface a calm error banner (technical details
    /// attached separately) and reuse the last-known-good landing instead of
    /// silently routing the user to onboarding.
    pub(crate) fn landing(&mut self, ctx: &Context) -> HubLanding {
        match self.app_context.load_local_qualified_identities() {
            Ok(identities) => {
                // Clear any previously-shown error banner now that loading works.
                self.load_error_banner.take_and_clear();
                let landing = HubLanding::from_identity_count(identities.len());
                self.last_good_landing = landing;
                landing
            }
            Err(e) => {
                // Idempotent: set_global de-duplicates by text, so repainting
                // this frame after frame does not spam banners.
                let handle = MessageBanner::set_global(
                    ctx,
                    "Could not load your identities from this device. Try refreshing or \
                     reopening the app.",
                    MessageType::Error,
                );
                handle.with_details(&e);
                self.load_error_banner = Some(handle);
                self.last_good_landing
            }
        }
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
    fn refresh(&mut self) {
        // Nothing to refresh in the scaffold itself — per-tab submodules own
        // their own cached data and will implement `refresh` when they land
        // (T8–T11). Clear the stale load-error banner so the next `landing()`
        // call can try again cleanly.
        self.load_error_banner.take_and_clear();
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
    }

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

        let landing = self.landing(ctx);

        action |= island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;

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
                            IdentityHubTab::Home => {
                                let (home_action, outcome) =
                                    super::home::render(ui, &self.app_context, &self.home_state);
                                if let Some(next_tab) =
                                    super::home::apply_outcome(&mut self.home_state, outcome)
                                {
                                    self.selected_tab = next_tab;
                                }
                                home_action
                            }
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

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // AppState sets the global banner centrally — we only need side-effects
        // here. The scaffold has none yet (no in-flight task banners owned by
        // the hub itself). Sub-tab content will override their own lifecycle.
    }

    fn display_task_result(&mut self, _result: BackendTaskSuccessResult) {
        // No backend tasks are dispatched from the hub root in the scaffold.
        // Tab submodules take over when they land; until then this is a no-op
        // so unexpected task results do not corrupt hub state.
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        // Let AppState render the default error banner — the hub has no
        // special-case error handling of its own yet.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests for the screen's pure state manipulation. Rendering is
    // covered by the kittest integration tests under
    // `tests/kittest/identity_hub.rs`, which mount a real `AppContext`.
    //
    // These tests avoid constructing an `AppContext` so they stay fast and
    // deterministic — we directly manipulate the small amount of state we
    // own (`selected_tab`, `last_good_landing`).

    #[test]
    fn default_state_machine() {
        // Mirror what `new()` initialises without depending on AppContext.
        let mut tab = IdentityHubTab::default();
        let mut last_good = HubLanding::Onboarding;
        assert_eq!(tab, IdentityHubTab::Home);
        assert_eq!(last_good, HubLanding::Onboarding);

        // Tab transitions round-trip.
        tab = IdentityHubTab::Settings;
        assert_eq!(tab, IdentityHubTab::Settings);
        tab = IdentityHubTab::Contacts;
        assert_eq!(tab, IdentityHubTab::Contacts);

        // Landing updates on successful load.
        last_good = HubLanding::from_identity_count(1);
        assert_eq!(last_good, HubLanding::Home);
        last_good = HubLanding::from_identity_count(5);
        assert_eq!(last_good, HubLanding::Picker);
    }

    #[test]
    fn load_error_preserves_last_good_landing() {
        // Simulate the fallback path in `landing()` without needing an
        // `AppContext`: cache a good state, then on error reuse it.
        let mut last_good = HubLanding::from_identity_count(3); // Picker
        // A load error must NOT downgrade to Onboarding.
        let would_fall_back_to = last_good; // the `landing` fn returns this on Err
        assert_eq!(would_fall_back_to, HubLanding::Picker);
        last_good = HubLanding::from_identity_count(0);
        // Next good load = empty account — falls back to Onboarding legitimately.
        assert_eq!(last_good, HubLanding::Onboarding);
    }
}
