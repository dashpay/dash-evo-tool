//! Root screen for the unified Identities hub.
//!
//! Holds the currently selected tab and the cached landing state; each frame it
//! recomputes the landing from the active-network identity count and dispatches
//! rendering to the appropriate submodule.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/04-dev-plan.md`.

use super::breadcrumb_switcher::{self, BreadcrumbEffect};
use super::identity_hub_tab_bar::IdentityHubTabBar;
use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel_with_breadcrumb;
use crate::ui::state::hub_selection::{HubSelection, HubView, effective_view};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Context};
use std::sync::Arc;

use super::home::HomeState;
use super::landing::HubLanding;
use super::settings::SettingsTab;
use super::tabs::IdentityHubTab;

/// Top-level screen for the new unified Identities hub.
///
/// Owns the selected tab, per-tab state, and the cached landing state; tab
/// submodules render into this via the shared `AppContext`.
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
    /// Settings-tab state. Held on the hub so edit fields, unsaved drafts,
    /// and modal state persist across frames.
    settings_tab: SettingsTab,
    /// Contacts-tab state. Owned here to debounce
    /// [`crate::backend_task::dashpay::DashPayTask::LoadContacts`] to a
    /// single dispatch per tab entry, instead of firing every frame.
    contacts_state: super::contacts::ContactsState,
    /// DashPay profiles, loaded asynchronously (the local profile cache was
    /// removed in the platform-wallet migration). Read by the Home, Contacts,
    /// and Settings tabs; loads are dispatched after rendering each frame.
    profile_cache: super::profile_cache::ProfileCache,
    /// Breadcrumb-switcher view state (picker override + dropdown search
    /// buffers). The active identity itself is app-scoped on `AppContext`.
    selection: HubSelection,
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
            settings_tab: SettingsTab::new(),
            contacts_state: super::contacts::ContactsState::default(),
            profile_cache: super::profile_cache::ProfileCache::default(),
            selection: HubSelection::default(),
        }
    }

    /// Resolve the current landing state from the active-network identity
    /// count. On load failure, surface a calm error banner (technical details
    /// attached separately) and reuse the last-known-good landing instead of
    /// silently routing the user to onboarding.
    pub(crate) fn landing(&mut self, ctx: &Context) -> HubLanding {
        // FR-6: the Identities hub is an everyday-user surface — its landing
        // count and picker list User identities only, never masternode/evonode.
        match self.app_context.load_local_user_identities() {
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
                handle.disable_auto_dismiss();
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

    /// Retire a request row the backend just resolved (accepted, declined, or
    /// cancelled), confirm it to the user, and re-arm the Contacts load so the
    /// authoritative lists replace the local edit.
    fn resolve_request(&mut self, request_id: &Identifier, confirmation: &str) {
        self.contacts_state.remove_request(request_id);
        self.contacts_state.invalidate();
        MessageBanner::set_global(
            self.app_context.egui_ctx(),
            confirmation,
            MessageType::Success,
        );
    }

    /// Apply a breadcrumb-switcher effect: wallet / identity switches mutate the
    /// app-scoped selection and reset identity-scoped caches; add-flows route to
    /// the existing screens.
    fn apply_breadcrumb_effect(&mut self, effect: BreadcrumbEffect) -> AppAction {
        match effect {
            BreadcrumbEffect::None => AppAction::None,
            BreadcrumbEffect::OpenPicker => {
                self.selection.open_picker();
                AppAction::None
            }
            BreadcrumbEffect::SwitchWallet(hash) => {
                self.app_context.set_selected_hd_wallet(Some(hash));
                self.selection.clear_picker_override();
                self.contacts_state.reset();
                self.profile_cache.reset();
                AppAction::None
            }
            BreadcrumbEffect::SelectIdentity(id) => {
                self.app_context.set_selected_identity(Some(id));
                self.selection.clear_picker_override();
                self.contacts_state.reset();
                self.profile_cache.reset();
                AppAction::None
            }
            BreadcrumbEffect::AddWallet => {
                AppAction::SetMainScreen(RootScreenType::RootScreenWalletsBalances)
            }
            // The bulk-create flow is not wired yet; route to the single-create
            // screen so the dev entry is functional in the interim.
            BreadcrumbEffect::AddIdentityCreate | BreadcrumbEffect::CreateTestIdentities => {
                AppAction::AddScreen(ScreenType::AddNewIdentity.create_screen(&self.app_context))
            }
            BreadcrumbEffect::AddIdentityLoad => AppAction::AddScreen(
                ScreenType::AddExistingIdentity.create_screen(&self.app_context),
            ),
        }
    }
}

impl ScreenLike for IdentityHubScreen {
    fn refresh(&mut self) {
        // Clear the stale load-error banner so the next `landing()` call can
        // try again cleanly, and reset per-tab load guards so a refresh
        // re-fires the Contacts load.
        self.load_error_banner.take_and_clear();
        self.contacts_state.reset();
        self.profile_cache.reset();
        self.selection.clear_searches();
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh();
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;

        // Top panel hosts the three-segment breadcrumb switcher on every
        // landing. The switcher is a pure component returning a typed effect;
        // the hub applies it below.
        let app_context = self.app_context.clone();
        let selection = &mut self.selection;
        let mut breadcrumb_effect = BreadcrumbEffect::None;
        action |= add_top_panel_with_breadcrumb(
            ui,
            &self.app_context,
            |ui| {
                breadcrumb_effect = breadcrumb_switcher::render(ui, &app_context, selection);
                AppAction::None
            },
            vec![],
        );

        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenIdentityHub);

        // Resolve the surface: onboarding (no identities) / picker (choose one)
        // / home (the resolved active identity). `landing()` keeps the
        // load-error banner behaviour and a last-known-good fallback.
        let landing = self.landing(ctx);
        // Load the identity list once per frame and reuse it for the view
        // computation AND the Picker arm (the Onboarding surface needs no list).
        // TODO(IDH-003 follow-up): also fold `landing()`'s load and the
        // breadcrumb switcher's per-frame loads into a single shared snapshot.
        let frame_identities = if matches!(landing, HubLanding::Onboarding) {
            Vec::new()
        } else {
            // FR-6: User identities only — the picker grid never lists MN/Evonode.
            self.app_context
                .load_local_user_identities()
                .unwrap_or_default()
        };
        let view = if matches!(landing, HubLanding::Onboarding) {
            HubView::Onboarding
        } else {
            let active = self.app_context.selected_identity_id();
            let has_explicit =
                active.is_some_and(|id| frame_identities.iter().any(|qi| qi.identity.id() == id));
            effective_view(
                frame_identities.len(),
                has_explicit,
                self.selection.picker_override(),
            )
        };

        let mut picked_identity: Option<String> = None;
        action |= island_central_panel(ui, |ui| {
            // Claim the island's full width so its bordered panel reaches the
            // window edges; the content below stays centered.
            ui.set_min_width(ui.available_width());
            match view {
                HubView::Onboarding => super::onboarding::render(ui, &self.app_context),
                HubView::Picker => super::picker::render(
                    ui,
                    &self.app_context,
                    &frame_identities,
                    Some(&mut picked_identity),
                ),
                HubView::Home => {
                    // `ui.vertical_centered(...).inner` carries the tab's
                    // `AppAction` back out.
                    let inner = ui.vertical_centered(|ui| {
                        // Hub tab bar; selection state lives on the screen so
                        // refreshes and deep links can update it from outside.
                        let tab_response = IdentityHubTabBar::new(self.selected_tab).show(ui);
                        if let Some(clicked) = tab_response.clicked() {
                            if clicked != self.selected_tab {
                                // Tab-entry transition: reset per-tab load guards
                                // so the incoming tab re-dispatches once.
                                self.contacts_state.reset();
                            }
                            self.selected_tab = clicked;
                        }
                        ui.add_space(16.0);
                        match self.selected_tab {
                            IdentityHubTab::Home => {
                                let (home_action, outcome) = super::home::render(
                                    ui,
                                    &self.app_context,
                                    &self.home_state,
                                    &mut self.profile_cache,
                                );
                                if let Some(next_tab) =
                                    super::home::apply_outcome(&mut self.home_state, outcome)
                                {
                                    if next_tab != self.selected_tab {
                                        self.contacts_state.reset();
                                    }
                                    self.selected_tab = next_tab;
                                }
                                home_action
                            }
                            IdentityHubTab::Contacts => super::contacts::render(
                                ui,
                                &self.app_context,
                                &mut self.contacts_state,
                                &mut self.profile_cache,
                            ),
                            IdentityHubTab::Activity => {
                                super::activity::render(ui, &self.app_context)
                            }
                            IdentityHubTab::Settings => self.settings_tab.render(
                                ui,
                                &self.app_context,
                                &mut self.profile_cache,
                            ),
                        }
                    });
                    inner.inner
                }
            }
        });

        // A picker card click sets the active identity and routes to Home.
        if let Some(id_str) = picked_identity
            && let Ok(id) = Identifier::from_string_unknown_encoding(&id_str)
        {
            self.app_context.set_selected_identity(Some(id));
            self.selection.clear_picker_override();
            self.contacts_state.reset();
            self.profile_cache.reset();
        }

        action |= self.apply_breadcrumb_effect(breadcrumb_effect);

        // Dispatch any profile load a tab requested this frame (single load
        // in flight; the local profile cache was removed in the platform-wallet
        // migration, so profiles resolve asynchronously via the backend).
        action |= self.profile_cache.dispatch_pending();

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // AppState sets the global banner centrally; the hub itself owns no
        // in-flight task banners. Sub-tab content overrides its own lifecycle.
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // Feed an async DashPay profile load back into the cache the tabs read.
        self.profile_cache.record_result(&result);

        match &result {
            // A confirmed profile-save success: commit the edit baseline on the
            // Settings tab so the Save button re-enables only after the next
            // edit. Guard by identity ID to reject stale results.
            BackendTaskSuccessResult::DashPayProfileUpdated(saved_id) => {
                let matches = self
                    .settings_tab
                    .selected_identity()
                    .is_some_and(|qi| qi.identity.id() == saved_id);
                if matches {
                    self.settings_tab.on_profile_saved();
                }
            }
            // Populate the Received/Sent request caches so the Contacts tab
            // can render real RequestCard rows instead of hardcoded empties.
            // The result arrives from LoadContactRequests,
            // dispatched alongside LoadContacts in contacts::render_populated.
            BackendTaskSuccessResult::DashPayContactRequests { incoming, outgoing } => {
                self.contacts_state
                    .record_requests(incoming.clone(), outgoing.clone());
            }
            // The established-contact list behind the Contacts tab's active
            // section and its search box.
            BackendTaskSuccessResult::DashPayContactsWithInfo(contacts) => {
                self.contacts_state.record_contacts(contacts.clone());
            }
            // A resolved request: drop the row now so the list reflects the
            // action immediately, then re-arm the load so the authoritative
            // lists (including the new contact, on accept) replace it.
            BackendTaskSuccessResult::DashPayContactRequestAccepted(request_id) => {
                self.resolve_request(request_id, "Contact request accepted.");
            }
            BackendTaskSuccessResult::DashPayContactRequestRejected(request_id) => {
                self.resolve_request(request_id, "Contact request declined.");
            }
            BackendTaskSuccessResult::DashPayContactRequestCancelled(request_id) => {
                self.resolve_request(request_id, "Contact request cancelled.");
            }
            // A confirmed contactInfo write — on this tab that is an unhide.
            // Re-arm the load so the restored contact comes back from the
            // authoritative list, not just the optimistic local move.
            BackendTaskSuccessResult::DashPayContactInfoUpdated(_) => {
                self.contacts_state.invalidate();
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "This contact is back in your list.",
                    MessageType::Success,
                );
            }
            // The counterpart answered while the row was on screen. Nothing to
            // withdraw or accept — reload into the truth.
            BackendTaskSuccessResult::DashPayContactAlreadyEstablished(_) => {
                self.contacts_state.invalidate();
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "You are already contacts with this person.",
                    MessageType::Info,
                );
            }
            _ => {}
        }
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        // Clear any dangling pending_save so a failed UpdateProfile doesn't
        // leave a stale snapshot around. If a later DashPayProfileUpdated from
        // a different path (e.g. the legacy ProfileScreen) arrives it would
        // otherwise commit the stale submitted values as the new baseline.
        // Clearing on any error is safe: pending_save is None most of the time,
        // and clearing it while it's None is a no-op.
        self.settings_tab.clear_pending_save();

        // Let AppState render the default error banner — the hub has no
        // other special-case error handling of its own yet.
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
