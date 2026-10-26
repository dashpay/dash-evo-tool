//! Root screen for the unified Identities hub.
//!
//! Holds the currently selected tab and the cached landing state; each frame it
//! recomputes the landing from the active-network identity count and dispatches
//! rendering to the appropriate submodule.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/04-dev-plan.md`.

use super::breadcrumb_switcher::{self, BreadcrumbEffect};
use super::identity_hub_tab_bar::IdentityHubTabBar;
use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dashpay::{ContactInfoField, ContactRequestKey};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel_with_breadcrumb;
use crate::ui::state::hub_profiles::HubProfiles;
use crate::ui::state::hub_selection::{HubSelection, HubView, effective_view};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Context};
use std::collections::{BTreeSet, HashMap};
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
    /// When a username status refresh was last dispatched (Unix ms). The cadence
    /// keys off dispatch time, so a result routed elsewhere never stalls it.
    usernames_refreshed_at: Option<u64>,
    /// The hub was just shown; the next frame applies the arrival refresh rule.
    usernames_arriving: bool,
    /// Identities this screen dispatched an unload for, each retained until its
    /// own result lands. Neither selection pointer survives that wait: a frame
    /// can render before the result arrives and reconcile both of them onto the
    /// identity that replaced this one.
    ///
    /// A set rather than one slot, because unloads overlap. The confirmation
    /// blocks input only while it is open; it closes on confirm and the task
    /// runs async, so the user can confirm A, switch to B and confirm B before
    /// A's result lands. A single slot would then hold B, and A's caches — its
    /// contact rows, load guard, pending tasks and cached profile — would never
    /// be reset, which is the original bug with an extra step.
    ///
    /// An entry whose removal failed is never consumed — a failure before
    /// delisting sends no result, and the screen's failure signal names no
    /// identity, so it cannot know which of several in-flight unloads to drop.
    /// The set therefore grows by one `Identifier` per failed attempt and never
    /// shrinks for one, for the life of the screen. Bounded by user action and
    /// gone when the screen is, so the size is not the concern; that a stale
    /// entry cannot misfire is, and it cannot: the check is keyed on the
    /// identities a *result* removed, never on this set, so a stale entry can
    /// only ever match a later result removing that same identity — precisely
    /// when the reset is wanted anyway.
    pending_unloads: BTreeSet<Identifier>,
    /// Contacts-tab state, built from the selected identity's DashPay
    /// snapshot.
    contacts_state: super::contacts::ContactsState,
    /// DashPay profiles of the identities on screen, re-read every frame.
    /// Read by the picker and the Home, Contacts, and Settings tabs.
    profiles: HubProfiles,
    /// A DashPay sync of the selected identity is due: after a refresh, an
    /// arrival, an identity switch, or entering the Contacts tab. Sent once
    /// the masternode list is synced.
    dashpay_sync_due: bool,
    avatar_cache: crate::ui::state::AvatarCache,
    pending_avatars: Vec<BackendTask>,
    /// Breadcrumb-switcher view state (picker override + dropdown search
    /// buffers). The active identity itself is app-scoped on `AppContext`.
    selection: HubSelection,
    pending_contact_info_tasks: HashMap<ContactInfoTaskKey, DashPayTask>,
    /// The dispatch each direct contact details write was sent under, and the
    /// network it runs on. Another screen can write the same contact while the
    /// hub's write waits behind it, so only an outcome for this dispatch
    /// settles the hub's write; one arriving after a network switch changes
    /// nothing the new network shows.
    contact_info_dispatches: HashMap<ContactInfoTaskKey, (u64, Network)>,
    /// Set by `display_backend_task_error` when the error belongs to another
    /// screen's write, so the context-free handler that follows it ignores it.
    foreign_contact_info_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ContactInfoTaskKey {
    Direct {
        identity_id: Identifier,
        contact_id: Identifier,
    },
    Request(ContactRequestKey),
}

impl IdentityHubScreen {
    /// Refresh username request status when due (USR-FR-021); schedules the next check.
    fn username_refresh_action(&mut self, ctx: &egui::Context) -> AppAction {
        // The refresh reads DPNS documents with proof verification; sent before
        // the masternode list is synced it gets the queried nodes banned. Wait,
        // keeping any arrival owed, until the list is ready.
        if !self.app_context.connection_status().masternodes_ready() {
            return AppAction::None;
        }
        let arriving = std::mem::take(&mut self.usernames_arriving);
        let network = self.app_context.network;
        let needs_refresh = self.app_context.username_requests_need_refresh();
        let now = crate::utils::time::now_ms();
        if needs_refresh {
            ctx.request_repaint_after(crate::model::dpns_usernames::pending_refresh_interval(
                network,
            ));
        }
        if !crate::model::dpns_usernames::username_refresh_due(
            now,
            self.usernames_refreshed_at,
            arriving,
            needs_refresh,
            network,
        ) {
            return AppAction::None;
        }
        self.usernames_refreshed_at = Some(now);
        AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::RefreshMyUsernameRequests,
        ))
    }

    /// The DashPay sync due for the selected identity, once the masternode
    /// list is synced: a pass reads proofs, and sent earlier it gets the
    /// queried nodes banned. Only Home and Contacts show what it syncs.
    fn dashpay_sync_action(&mut self, rendered_tab: Option<IdentityHubTab>) -> AppAction {
        if !self.dashpay_sync_due
            || !matches!(
                rendered_tab,
                Some(IdentityHubTab::Home | IdentityHubTab::Contacts)
            )
            || !self.app_context.connection_status().masternodes_ready()
        {
            return AppAction::None;
        }
        self.dashpay_sync_due = false;
        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(DashPayTask::SyncNow {
            identity: self.app_context.resolve_selected_identity(),
        })))
    }

    /// Whether an avatar result still belongs to an outstanding hub request.
    pub(crate) fn is_waiting_for_avatar(&self, url: &str) -> bool {
        self.avatar_cache.awaits_result(url)
    }

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
            usernames_refreshed_at: None,
            usernames_arriving: true,
            pending_unloads: BTreeSet::new(),
            contacts_state: super::contacts::ContactsState::default(),
            profiles: HubProfiles::default(),
            dashpay_sync_due: true,
            avatar_cache: crate::ui::state::AvatarCache::default(),
            pending_avatars: Vec::new(),
            selection: HubSelection::default(),
            pending_contact_info_tasks: HashMap::new(),
            contact_info_dispatches: HashMap::new(),
            foreign_contact_info_error: false,
        }
    }

    fn capture_contact_info_update(&mut self, action: &AppAction) {
        let AppAction::BackendTask(BackendTask::DashPayTask(task)) = action else {
            return;
        };
        if let Some(key) = contact_info_task_key(task) {
            self.pending_contact_info_tasks
                .insert(key, task.as_ref().clone());
        }
    }

    /// Send a direct contact details write under its own dispatch context and
    /// remember it; see [`Self::contact_info_dispatches`].
    fn bind_contact_info_dispatch(&mut self, action: AppAction) -> AppAction {
        let AppAction::BackendTask(task) = action else {
            return action;
        };
        let context = BackendTaskContext::for_dispatch(&task);
        let (Some((identity_id, contact_id)), Some(dispatch_id)) =
            (context.dashpay_contact_info_target(), context.dispatch_id())
        else {
            return AppAction::BackendTask(task);
        };
        let key = ContactInfoTaskKey::Direct {
            identity_id,
            contact_id,
        };
        self.contact_info_dispatches
            .insert(key, (dispatch_id, self.app_context.network));
        AppAction::BackendTaskWithContext { task, context }
    }

    /// The network the hub's write to `contact_id` runs on: the one recorded
    /// at dispatch, else the network on screen.
    fn contact_info_write_network(
        &self,
        identity_id: &Identifier,
        contact_id: &Identifier,
    ) -> Network {
        self.contact_info_dispatches
            .get(&ContactInfoTaskKey::Direct {
                identity_id: *identity_id,
                contact_id: *contact_id,
            })
            .map_or(self.app_context.network, |(_, network)| *network)
    }

    /// Retire a settled or abandoned write's records together.
    fn forget_contact_info_task(&mut self, key: &ContactInfoTaskKey) -> Option<DashPayTask> {
        self.contact_info_dispatches.remove(key);
        self.pending_contact_info_tasks.remove(key)
    }

    /// Whether `context` is the dispatch of the hub's own write for its contact.
    fn is_own_contact_info_dispatch(&self, context: &BackendTaskContext) -> bool {
        context
            .dashpay_contact_info_target()
            .and_then(|(identity_id, contact_id)| {
                self.contact_info_dispatches
                    .get(&ContactInfoTaskKey::Direct {
                        identity_id,
                        contact_id,
                    })
            })
            .is_some_and(|(dispatch_id, _)| context.dispatch_id() == Some(*dispatch_id))
    }

    /// Remember the identity an unload was dispatched for.
    ///
    /// The removal is answered frames later, and `SettingsTab::ensure_selected`
    /// reconciles its retained identity to the effective one on every frame in
    /// between — so by the time the result lands, both the explicit selection
    /// and the tab's identity can name the replacement, and nothing left in the
    /// screen would say which identity's caches are now stale. This is that
    /// record, and the reconciler cannot move it.
    fn capture_unload_dispatch(&mut self, action: &AppAction) {
        if let AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RemoveIdentity {
            identity_id,
        })) = action
        {
            self.pending_unloads.insert(*identity_id);
        }
    }

    fn reset_contacts_for_identity_change(&mut self) {
        self.home_state = HomeState::default();
        self.contacts_state.reset_for_identity_change();
        self.dashpay_sync_due = true;
        // A write still running keeps its guard across the change, so it keeps
        // the records its outcome is matched against too.
        let detail = self.contacts_state.detail();
        let live = |key: &ContactInfoTaskKey| {
            matches!(key, ContactInfoTaskKey::Direct { identity_id, contact_id }
                if detail.is_writing(identity_id, contact_id))
        };
        self.pending_contact_info_tasks.retain(|key, _| live(key));
        self.contact_info_dispatches.retain(|key, _| live(key));
    }

    /// Clear the stale load-error banner so the next `landing()` call can try
    /// again cleanly, and ask for a DashPay sync. With `keep_shown`, the
    /// contacts and avatars stay on screen while it runs.
    fn reload(&mut self, keep_shown: bool) {
        self.load_error_banner.take_and_clear();
        if keep_shown {
            self.contacts_state.search_mut().clear();
            self.avatar_cache.reload_kept();
        } else {
            self.contacts_state.reset();
            self.avatar_cache.invalidate();
        }
        self.dashpay_sync_due = true;
        self.selection.clear_searches();
        self.pending_avatars.clear();
    }

    /// Reset all identity-scoped state after the owning application context
    /// changes. Live paid actions remain guarded, and stale guards are pruned
    /// during the contacts reset.
    pub(crate) fn reset_for_context_change(&mut self) {
        self.reset_contacts_for_identity_change();
        self.settings_tab = SettingsTab::new();
        self.selection.clear_picker_override();
        self.selection.clear_searches();
        self.avatar_cache.invalidate();
        self.pending_avatars.clear();
        self.load_error_banner.take_and_clear();
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

    /// Show `contacts` for the identity on screen, as a sync would.
    #[cfg(any(test, feature = "testing"))]
    pub fn show_contacts_for_test(&mut self, contacts: Vec<crate::model::dashpay::ContactData>) {
        self.contacts_state.record_contacts_for_test(contacts);
    }

    /// Retire a request row the backend just resolved (accepted, declined, or
    /// cancelled) and confirm it to the user.
    fn resolve_request(&mut self, request: &ContactRequestKey, confirmation: &str) {
        self.pending_contact_info_tasks
            .remove(&ContactInfoTaskKey::Request(*request));
        self.contacts_state.remove_request(request);
        MessageBanner::set_global(
            self.app_context.egui_ctx(),
            confirmation,
            MessageType::Success,
        );
    }

    pub(crate) fn handle_contact_request_result(
        &mut self,
        result: &BackendTaskSuccessResult,
    ) -> bool {
        match result {
            BackendTaskSuccessResult::DashPayContactRequestAccepted(request) => {
                self.resolve_request(request, "Contact request accepted.");
            }
            BackendTaskSuccessResult::DashPayContactRequestRejected(request) => {
                self.resolve_request(request, "Contact request declined.");
            }
            BackendTaskSuccessResult::DashPayContactRequestCancelled(request) => {
                self.resolve_request(request, "Contact request cancelled.");
            }
            BackendTaskSuccessResult::DashPayContactAlreadyEstablished(request) => {
                self.pending_contact_info_tasks
                    .remove(&ContactInfoTaskKey::Request(*request));
                self.contacts_state.remove_request(request);
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "You are already contacts with this person.",
                    MessageType::Info,
                );
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn handle_contact_request_error(&mut self, error: &TaskError) -> bool {
        if !matches!(error, TaskError::DashPayContactRequestActionFailed { .. }) {
            return false;
        }
        if let Some(key) = contact_info_error_key(error) {
            self.forget_contact_info_task(&key);
        }
        release_request_guard_for_error(&mut self.contacts_state, error);
        true
    }

    /// Settle a failed direct contact details write: drop the pending write
    /// and release its guard. The error banner tells the user.
    pub(crate) fn handle_contact_info_error(&mut self, error: &TaskError) -> bool {
        // Another screen's write failed; the hub's own write is still running.
        if std::mem::take(&mut self.foreign_contact_info_error)
            || !matches!(error, TaskError::DashPayContactInfoActionFailed { .. })
        {
            return false;
        }
        if let TaskError::DashPayContactInfoActionFailed {
            identity_id,
            contact_id,
            ..
        } = error
        {
            let network = self.contact_info_write_network(identity_id, contact_id);
            self.contacts_state
                .release_contact_info_write(network, identity_id, contact_id);
        }
        if let Some(key) = contact_info_error_key(error) {
            self.forget_contact_info_task(&key);
        }
        false
    }

    /// Show all User identities, clearing the selected wallet filter.
    pub(crate) fn open_picker(&mut self) {
        self.app_context.set_selected_hd_wallet(None);
        self.reset_contacts_for_identity_change();
        self.selection.open_picker();
    }

    /// Apply a breadcrumb-switcher effect: wallet / identity switches mutate the
    /// app-scoped selection and reset identity-scoped caches; add-flows route to
    /// the existing screens.
    fn apply_breadcrumb_effect(&mut self, effect: BreadcrumbEffect) -> AppAction {
        match effect {
            BreadcrumbEffect::None => AppAction::None,
            BreadcrumbEffect::OpenPicker => {
                self.open_picker();
                AppAction::None
            }
            BreadcrumbEffect::SwitchWallet(hash) => {
                self.app_context.set_selected_hd_wallet(hash);
                self.selection.open_picker();
                self.reset_contacts_for_identity_change();
                AppAction::None
            }
            BreadcrumbEffect::SelectIdentity(id) => {
                self.app_context.set_selected_identity(Some(id));
                self.selection.clear_picker_override();
                self.reset_contacts_for_identity_change();
                AppAction::None
            }
            BreadcrumbEffect::AddWallet => {
                AppAction::SetMainScreen(RootScreenType::RootScreenWalletsBalances)
            }
            BreadcrumbEffect::AddIdentityCreate => {
                AppAction::AddScreen(crate::ui::Screen::AddNewIdentityScreen(
                    super::add_new_identity_screen::AddNewIdentityScreen::new_with_wallet(
                        &self.app_context,
                        self.app_context.selected_wallet_hash(),
                    ),
                ))
            }
            BreadcrumbEffect::AddIdentityLoad => AppAction::AddScreen(
                ScreenType::AddExistingIdentity.create_screen(&self.app_context),
            ),
        }
    }
}

impl ScreenLike for IdentityHubScreen {
    fn refresh(&mut self) {
        self.reload(false);
    }

    /// Returning keeps the profile and contacts already shown and reloads them
    /// behind; lists of another identity are dropped as the hub renders.
    fn refresh_on_arrival(&mut self) {
        self.reload(true);
        self.usernames_arriving = true;
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
            vec![(
                "Add",
                DesiredAppAction::Menu(breadcrumb_switcher::add_identity_menu_items()),
            )],
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
            match self.app_context.selected_wallet_hash() {
                Some(hash) => self
                    .app_context
                    .load_local_qualified_identities_for_wallet(&hash)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|identity| {
                        identity.identity_type
                            == crate::model::qualified_identity::IdentityType::User
                    })
                    .collect(),
                None => self
                    .app_context
                    .load_local_user_identities()
                    .unwrap_or_default(),
            }
        };
        self.profiles.read(
            &self.app_context,
            frame_identities
                .iter()
                .map(|identity| identity.identity.id()),
        );
        let view = if self.selection.picker_override()
            || self.app_context.selected_wallet_hash().is_some() && frame_identities.is_empty()
        {
            HubView::Picker
        } else if matches!(landing, HubLanding::Onboarding) {
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
        // The tab whose content this frame rendered, if the hub shows one.
        let mut rendered_tab: Option<IdentityHubTab> = None;
        action |= island_central_panel(ui, |ui| {
            // Claim the island's full width so its bordered panel reaches the
            // window edges; each view lays out its own content inside it.
            ui.set_min_width(ui.available_width());
            match view {
                HubView::Onboarding => super::onboarding::render(ui, &self.app_context),
                HubView::Picker => super::picker::render(
                    ui,
                    &self.app_context,
                    &frame_identities,
                    &self.profiles,
                    &mut self.avatar_cache,
                    &mut self.pending_avatars,
                    Some(&mut picked_identity),
                ),
                HubView::Home => {
                    // One roster read per frame serves the header and every tab.
                    let selected = self.app_context.resolve_selected_identity();
                    // Share the island's available width across the header and
                    // tabs; each tab handles its own responsive columns.
                    let inner = ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        let (header_action, header_outcome) = match &selected {
                            Some(identity) => super::home::render_header(
                                ui,
                                &self.app_context,
                                identity,
                                &self.profiles,
                                &mut self.avatar_cache,
                                &mut self.pending_avatars,
                            ),
                            None => (AppAction::None, super::home::HomeOutcome::None),
                        };
                        if let Some(tab) =
                            super::home::apply_outcome(&mut self.home_state, header_outcome)
                        {
                            self.selected_tab = tab;
                            self.settings_tab.focus_section(header_outcome);
                        }
                        ui.add_space(16.0);
                        // Hub tab bar; selection state lives on the screen so
                        // refreshes and deep links can update it from outside.
                        let tab_response = IdentityHubTabBar::new(self.selected_tab).show(ui);
                        if let Some(clicked) = tab_response.clicked() {
                            if clicked != self.selected_tab {
                                // Tab entry starts the lists afresh.
                                self.contacts_state.reset();
                                self.dashpay_sync_due |= clicked == IdentityHubTab::Contacts;
                            }
                            self.selected_tab = clicked;
                        }
                        ui.add_space(16.0);
                        rendered_tab = Some(self.selected_tab);
                        let mut tab_action = match self.selected_tab {
                            IdentityHubTab::Home => {
                                let (home_action, outcome) = match &selected {
                                    Some(identity) => super::home::render(
                                        ui,
                                        &self.app_context,
                                        identity,
                                        &mut self.home_state,
                                        &self.profiles,
                                        &mut self.contacts_state,
                                    ),
                                    None => (AppAction::None, super::home::HomeOutcome::None),
                                };
                                if let Some(next_tab) =
                                    super::home::apply_outcome(&mut self.home_state, outcome)
                                {
                                    if let super::home::HomeOutcome::GoToContact(contact_id) =
                                        outcome
                                    {
                                        self.contacts_state.search_mut().clear();
                                        self.contacts_state.select_contact(&contact_id);
                                    } else if next_tab != self.selected_tab {
                                        self.contacts_state.reset();
                                    }
                                    self.selected_tab = next_tab;
                                    self.settings_tab.focus_section(outcome);
                                }
                                home_action
                            }
                            IdentityHubTab::Contacts => super::contacts::render(
                                ui,
                                &self.app_context,
                                selected.as_ref(),
                                &mut self.contacts_state,
                                &self.profiles,
                                &mut self.avatar_cache,
                                &mut self.pending_avatars,
                            ),
                            IdentityHubTab::Activity => {
                                super::activity::render(ui, &self.app_context)
                            }
                            IdentityHubTab::Settings => self.settings_tab.render(
                                ui,
                                &self.app_context,
                                selected.as_ref(),
                                &self.profiles,
                            ),
                        };
                        tab_action |= header_action;
                        tab_action
                    });
                    inner.inner
                }
            }
        });
        self.capture_contact_info_update(&action);
        action = self.bind_contact_info_dispatch(action);
        self.capture_unload_dispatch(&action);

        // A picker card click sets the active identity and routes to Home.
        if let Some(id_str) = picked_identity
            && let Ok(id) = Identifier::from_string_unknown_encoding(&id_str)
        {
            self.app_context.set_selected_identity(Some(id));
            self.selection.clear_picker_override();
            self.reset_contacts_for_identity_change();
        }

        action |= self.apply_breadcrumb_effect(breadcrumb_effect);
        // The top-bar "Add" menu and the picker's add card emit
        // `AppAction::Custom` commands; route them like the identity pill's
        // add entries.
        if let Some(effect) = breadcrumb_switcher::add_identity_command_effect(&action) {
            action = self.apply_breadcrumb_effect(effect);
        }

        // Every part of the frame above has had its say; this is where the
        // action that leaves the frame is settled. Background work is asked
        // for only when nothing produced an action: a click anywhere on the
        // screen keeps its frame, and nothing below replaces an action.
        if matches!(action, AppAction::None) {
            action = self.dashpay_sync_action(rendered_tab);
        }
        if matches!(action, AppAction::None)
            && let Some(task) = self.pending_avatars.pop()
        {
            action = AppAction::BackendTask(task);
        }
        if matches!(action, AppAction::None) {
            action = self.username_refresh_action(ctx);
        }
        if matches!(action, AppAction::None)
            && let Some((task, context)) = self.settings_tab.usernames_card_mut().recovery_check()
        {
            action = AppAction::BackendTaskWithContext { task, context };
        }
        action
    }

    fn accepts_legacy_recovery_result(
        &self,
        context: &BackendTaskContext,
        completed: bool,
    ) -> bool {
        self.settings_tab
            .accepts_legacy_recovery_result(context, completed)
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        if let BackendTaskSuccessResult::DashPayProfileUpdated(snapshot) = &result {
            handle_profile_updated(&self.app_context, &mut self.settings_tab, context, snapshot);
            return;
        }
        // Another screen's write: the snapshot shows it, and that screen
        // reports it.
        if matches!(
            result,
            BackendTaskSuccessResult::DashPayContactInfoUpdated { .. }
        ) && !self.is_own_contact_info_dispatch(context)
        {
            return;
        }
        self.display_task_result(result);
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // AppState sets the global banner centrally; the hub itself owns no
        // in-flight task banners. Sub-tab content overrides its own lifecycle.
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // The record a finished restore wrote is re-read by the Settings tab on
        // its next frame, so the offer's own outcome needs no follow-up here.
        if let Some(offer) = self.settings_tab.usernames_card_mut().recovery_mut() {
            offer.absorb_result(self.app_context.egui_ctx(), &result);
        }
        if matches!(
            result,
            BackendTaskSuccessResult::MyUsernameRequestsRefreshed
                | BackendTaskSuccessResult::UsernamePreferencesSaved
        ) {
            return;
        }
        if let BackendTaskSuccessResult::DashPayAvatar { url, bytes } = result {
            if self.is_waiting_for_avatar(&url) {
                self.avatar_cache.store(url, bytes);
            }
            return;
        }

        if self.handle_contact_request_result(&result) {
            return;
        }

        match &result {
            // A confirmed contactInfo write — an unhide or a private-note
            // save. The snapshot already holds it.
            BackendTaskSuccessResult::DashPayContactInfoUpdated {
                identity,
                contact_id,
                published,
            } => {
                let key = ContactInfoTaskKey::Direct {
                    identity_id: *identity,
                    contact_id: *contact_id,
                };
                let network = self.contact_info_write_network(identity, contact_id);
                self.contacts_state
                    .finish_contact_info_write(network, identity, contact_id);
                if let Some(task) = self.forget_contact_info_task(&key) {
                    let (message, kind) = if *published {
                        (contact_info_saved_message(&task), MessageType::Success)
                    } else {
                        (
                            super::contacts::DETAILS_SAVED_ON_THIS_DEVICE,
                            MessageType::Info,
                        )
                    };
                    MessageBanner::set_global(self.app_context.egui_ctx(), message, kind);
                }
            }
            // AppState delivers removals even while this root is hidden and owns the banner.
            BackendTaskSuccessResult::RemovedIdentities {
                network,
                identity_ids,
                ..
            } => {
                if *network != self.app_context.network {
                    return;
                }
                // Two separate questions with two separate answers. The
                // app-wide selection is dropped only when it is the thing that
                // was removed, so the hub falls back deliberately instead of
                // resolving around a pointer to a deleted identity.
                let explicit_selection = self.app_context.selected_identity_id();
                if explicit_selection.is_some_and(|active| identity_ids.contains(&active)) {
                    self.app_context.set_selected_identity(None);
                }
                // The caches follow the *effective* identity, which is a
                // fallback whenever no selection is stored, so they are reset
                // on the wider condition — contacts, pending contact
                // confirmations, and the profile cache — matching every other
                // identity-switch path, so a resolve to the next identity next
                // frame does not inherit state that still belongs to the one
                // just removed.
                if removal_invalidates_identity_caches(
                    explicit_selection,
                    self.settings_tab
                        .selected_identity()
                        .map(|identity| identity.identity.id()),
                    &self.pending_unloads,
                    identity_ids,
                ) {
                    self.reset_contacts_for_identity_change();
                }
                // Only the identities this result answers for; another unload
                // still in flight keeps its own record.
                self.pending_unloads
                    .retain(|pending| !identity_ids.contains(pending));
            }
            _ => {}
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        self.foreign_contact_info_error = context.dashpay_contact_info_target().is_some()
            && !self.is_own_contact_info_dispatch(context);
        // Wrapped errors are settled by `handle_contact_info_error`.
        if !self.foreign_contact_info_error
            && !matches!(error, TaskError::DashPayContactInfoActionFailed { .. })
            && let Some((identity_id, contact_id)) = context.dashpay_contact_info_target()
        {
            let network = self.contact_info_write_network(&identity_id, &contact_id);
            self.forget_contact_info_task(&ContactInfoTaskKey::Direct {
                identity_id,
                contact_id,
            });
            self.contacts_state
                .release_contact_info_write(network, &identity_id, &contact_id);
        }
        self.settings_tab.clear_pending_save(context);
        if let Some(offer) = self.settings_tab.usernames_card_mut().recovery_mut() {
            offer.absorb_error(context);
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        // The status refresh runs in the background; a failure keeps the stored
        // status and retries on the next cadence tick instead of raising a banner.
        if matches!(error, TaskError::UsernameRequestRefreshFailed { .. }) {
            return true;
        }
        if self.handle_contact_request_error(error) {
            return false;
        }
        if self.handle_contact_info_error(error) {
            return true;
        }
        release_request_guard_for_error(&mut self.contacts_state, error);

        // Let AppState render the default error banner — the hub has no
        // other special-case error handling of its own yet.
        false
    }
}

/// Release the request-card guard a failed contact action names. Both a task-level
/// failure and a migration-gate refusal arrive as
/// [`TaskError::DashPayContactRequestActionFailed`], carrying the request, so
/// only that request's guard is released; every other guard stays protected until
/// its own typed outcome arrives — a paid action still in flight must never have
/// its row re-enabled by an unrelated failure.
fn release_request_guard_for_error(state: &mut super::contacts::ContactsState, error: &TaskError) {
    if let TaskError::DashPayContactRequestActionFailed { request, .. } = error {
        state.release_request(request);
    }
}

/// Whether a captured contactInfo write is a private-note save rather than an
/// unhide.
fn is_note_save(task: &DashPayTask) -> bool {
    matches!(
        task,
        DashPayTask::UpdateContactInfo { update, .. }
            if matches!(update.note, ContactInfoField::Replace(_))
    )
}

/// Success banner for a confirmed direct contactInfo write.
fn contact_info_saved_message(task: &DashPayTask) -> &'static str {
    if is_note_save(task) {
        super::contacts::NOTE_SAVED
    } else {
        "This contact is back in your list."
    }
}

fn contact_info_task_key(task: &DashPayTask) -> Option<ContactInfoTaskKey> {
    match task {
        DashPayTask::UpdateContactInfo {
            identity,
            contact_id,
            ..
        } => Some(ContactInfoTaskKey::Direct {
            identity_id: identity.identity.id(),
            contact_id: *contact_id,
        }),
        DashPayTask::RejectContactRequest {
            identity,
            counterparty_id,
        }
        | DashPayTask::CancelContactRequest {
            identity,
            counterparty_id,
        } => Some(ContactInfoTaskKey::Request(ContactRequestKey {
            owner: identity.identity.id(),
            counterparty: *counterparty_id,
        })),
        _ => None,
    }
}

fn contact_info_error_key(error: &TaskError) -> Option<ContactInfoTaskKey> {
    match error {
        TaskError::DashPayContactInfoActionFailed {
            identity_id,
            contact_id,
            ..
        } => Some(ContactInfoTaskKey::Direct {
            identity_id: *identity_id,
            contact_id: *contact_id,
        }),
        TaskError::DashPayContactRequestActionFailed { request, .. } => {
            Some(ContactInfoTaskKey::Request(*request))
        }
        _ => None,
    }
}

/// Whether a completed removal leaves the Hub's identity-scoped caches holding
/// data for an identity that no longer exists.
///
/// The caches follow the *effective* identity — the explicit selection when
/// there is one, a fallback otherwise — so the explicit pointer alone does not
/// answer this: a single-identity account never stores one, and its unload
/// would reset nothing. Re-resolving the fallback cannot answer it either,
/// because by the time this result arrives the removed identity is gone from
/// storage and the resolver names whichever identity replaced it.
///
/// Nor is the Settings tab's retained identity enough on its own, which is
/// what an earlier version of this got wrong: it is itself reconciled to the
/// effective identity on every frame, and a frame can render between the
/// dispatch and the result, leaving all three of storage, the selection and
/// the tab naming the replacement. Only the id captured when the unload was
/// dispatched survives that, so it is checked alongside the two live pointers
/// — any of the three naming a removed id invalidates the caches.
///
/// `dispatched_unloads` is a set because unloads overlap: a result answers for
/// one of them while the others are still in flight, so matching against a
/// single remembered id would miss every removal but the most recent.
fn removal_invalidates_identity_caches(
    explicit_selection: Option<Identifier>,
    settings_identity: Option<Identifier>,
    dispatched_unloads: &BTreeSet<Identifier>,
    removed: &[Identifier],
) -> bool {
    removed.iter().any(|identity| {
        explicit_selection == Some(*identity)
            || settings_identity == Some(*identity)
            || dispatched_unloads.contains(identity)
    })
}

pub(super) fn handle_profile_updated(
    app_context: &AppContext,
    settings: &mut SettingsTab,
    context: &BackendTaskContext,
    snapshot: &crate::model::dashpay::ProfileSnapshot,
) {
    if snapshot.network != app_context.network {
        return;
    }
    if !app_context.profile_snapshot_is_current(snapshot) {
        settings.retry_stale_profile_save(context);
        return;
    }
    if settings.owns_profile_save(context) && settings.on_profile_saved().is_some() {
        MessageBanner::set_global(
            app_context.egui_ctx(),
            crate::ui::identity::settings::PROFILE_SAVED,
            MessageType::Success,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    use crate::ui::state::contacts_view::ContactsState;
    use crate::utils::egui_mpsc::SenderAsync;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::collections::BTreeMap;

    // Most tests exercise pure screen state. The identity-result regression
    // uses a throwaway AppContext because selection fallback is the behavior
    // under test. Rendering stays covered by the kittest identity-hub tests.

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

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32-byte identifier")
    }

    /// The request between the owner `id(100)` and `id(counterparty)`.
    fn req(counterparty: u8) -> ContactRequestKey {
        ContactRequestKey {
            owner: id(100),
            counterparty: id(counterparty),
        }
    }

    #[test]
    fn stale_profile_success_does_not_show_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::context::test_support::test_app_context(dir.path());
        let ctx = app.egui_ctx();
        let mut settings = SettingsTab::new();

        handle_profile_updated(
            &app,
            &mut settings,
            &BackendTaskContext::Other,
            &crate::model::dashpay::ProfileSnapshot {
                network: app.network,
                owner: id(1),
                revision: 0,
                profile: None,
            },
        );

        assert!(
            !MessageBanner::has_global(ctx),
            "a stale result has no pending save to confirm"
        );
    }

    async fn wired_test_context() -> (tempfile::TempDir, Arc<AppContext>) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, context.egui_ctx().clone());
        context
            .ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend");
        (temp_dir, context)
    }

    fn seed_user_identity(context: &AppContext, byte: u8) -> Identifier {
        let identity =
            Identity::create_basic_identity(id(byte), PlatformVersion::latest()).expect("identity");
        let identity_id = identity.id();
        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::PendingCreation,
            network: Network::Testnet,
        };
        context
            .insert_local_qualified_identity(&qualified_identity, &None)
            .expect("seed identity");
        identity_id
    }

    /// Proof-verifying reads sent before the masternode list is synced fail
    /// locally, and the SDK bans every node they reach for that failure. Home
    /// therefore holds its contact, payment-history (which syncs DashPay
    /// first) and profile reads until the network is ready, while still
    /// showing that contacts are loading.
    /// The hub's automatic username-status refresh reads DPNS documents with
    /// proof verification, so it waits for the masternode list like Home's own
    /// reads; otherwise it can ban the endpoints contact recovery needs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_username_refresh_waits_until_masternodes_are_ready() {
        let (_dir, context) = wired_test_context().await;
        let owner = seed_user_identity(&context, 0x60);
        context.set_selected_identity(Some(owner));
        context.connection_status().set_masternodes_ready(false);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, usize, usize)| {
                    state.1 += 1;
                    if let AppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::RefreshMyUsernameRequests,
                    )) = state.0.ui(ui)
                    {
                        state.2 += 1;
                    }
                },
                (IdentityHubScreen::new(&context), 0, 0),
            );
        harness.run_steps(6);
        assert!(harness.state().1 >= 6, "the hub rendered every frame");
        assert_eq!(harness.state().2, 0, "no refresh before readiness");

        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(10);
        assert_eq!(
            harness.state().2,
            1,
            "one refresh once the network is ready"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn home_holds_its_proof_reads_until_masternodes_are_ready() {
        let (_dir, context) = wired_test_context().await;
        let owner = seed_user_identity(&context, 0x5F);
        context.set_selected_identity(Some(owner));
        context.connection_status().set_masternodes_ready(false);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Vec<&'static str>)| {
                    let tasks = match state.0.ui(ui) {
                        AppAction::BackendTask(task)
                        | AppAction::BackendTaskWithContext { task, .. } => vec![task],
                        AppAction::BackendTasks(tasks, _) => tasks,
                        _ => Vec::new(),
                    };
                    for task in tasks {
                        if let BackendTask::DashPayTask(task) = task
                            && matches!(*task, DashPayTask::SyncNow { identity: Some(_) })
                        {
                            state.1.push("sync");
                        }
                    }
                },
                (IdentityHubScreen::new(&context), Vec::new()),
            );
        harness.run_steps(4);
        assert_eq!(
            harness.state().1,
            Vec::<&str>::new(),
            "no read before readiness"
        );
        assert!(
            harness.state().0.dashpay_sync_due,
            "Home asked for a sync and was held, not skipped"
        );
        {
            use egui_kittest::kittest::Queryable;
            assert!(harness.query_by_label("Loading contacts…").is_some());
        }

        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(6);
        assert_eq!(
            harness.state().1,
            ["sync"],
            "one sync of the selected identity once the network is ready"
        );
    }

    /// On an updated install the hub sends the Usernames card's check of the
    /// previous version's data exactly once and accepts its answer, so the
    /// restore offer can reach an identity whose key is not on this device.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hub_sends_the_usernames_card_recovery_check_once_and_accepts_its_answer() {
        let (_dir, context) = wired_test_context().await;
        crate::database::test_helpers::LegacyIdentityFixture::new(
            [0x31; 32],
            Some(vec![0x01]),
            "testnet",
        )
        .insert(&context.db.locked_conn())
        .expect("stage a legacy identity row");
        let identity = crate::context::test_support::user_identity_with_signing_key(
            id(0x31),
            Network::Testnet,
            false,
        );
        context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed identity");
        context.set_selected_identity(Some(identity.identity.id()));

        let mut hub = IdentityHubScreen::new(&context);
        hub.select_tab(IdentityHubTab::Settings);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Vec<BackendTaskContext>)| {
                    if let AppAction::BackendTaskWithContext {
                        task: BackendTask::IdentityTask(IdentityTask::CheckLegacyRecovery { .. }),
                        context,
                    } = state.0.ui(ui)
                    {
                        state.1.push(context);
                    }
                },
                (hub, Vec::new()),
            );
        harness.run_steps(12);

        let (hub, checks) = harness.state();
        assert_eq!(checks.len(), 1);
        assert!(hub.accepts_legacy_recovery_result(&checks[0], false));
        assert!(!hub.accepts_legacy_recovery_result(
            &BackendTaskContext::LegacyRecoveryCheck(id(0x32)),
            false
        ));
    }

    /// Publish `profile` as `owner`'s synced profile.
    async fn sync_profile(
        context: &AppContext,
        owner: &Identifier,
        profile: platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile,
    ) {
        let backend = context.wallet_backend().unwrap();
        backend
            .dashpay_set_profile(owner, Some(profile))
            .await
            .unwrap();
        backend.refresh_dashpay_snapshots().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_synced_profile_rebases_the_settings_editor_before_save() {
        use egui_kittest::{Harness, kittest::Queryable};
        use platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile;
        let (_dir, context) = wired_test_context().await;
        let identity = seed_cached_profile(&context).await;
        let owner = identity.identity.id();
        for dirty in [false, true] {
            sync_profile(
                &context,
                &owner,
                DashPayProfile {
                    display_name: Some("Cached profile".into()),
                    avatar_url: Some("https://example.com/cached.png".into()),
                    bio: None,
                    public_message: None,
                    avatar_hash: None,
                    avatar_fingerprint: None,
                },
            )
            .await;
            let hub = IdentityHubScreen::new(&context);
            let mut editor = Harness::builder()
                .with_size(egui::vec2(1100.0, 1100.0))
                .build_ui_state(
                    |ui, state: &mut (IdentityHubScreen, Option<BackendTask>)| {
                        let hub = &mut state.0;
                        hub.profiles.read(&hub.app_context, [owner]);
                        if let AppAction::BackendTaskWithContext { task, .. } =
                            hub.settings_tab.render(
                                ui,
                                &hub.app_context,
                                hub.app_context.resolve_selected_identity().as_ref(),
                                &hub.profiles,
                            )
                        {
                            state.1 = Some(task);
                        }
                    },
                    (hub, None),
                );
            editor.run();
            let type_draft =
                |editor: &mut Harness<'_, (IdentityHubScreen, Option<BackendTask>)>| {
                    editor
                        .query_all_by_role(egui::accesskit::Role::TextInput)
                        .next()
                        .unwrap()
                        .focus();
                    editor.run();
                    editor
                        .query_all_by_role(egui::accesskit::Role::TextInput)
                        .next()
                        .unwrap()
                        .type_text(" draft");
                    editor.run();
                };
            if dirty {
                type_draft(&mut editor);
            }
            sync_profile(
                &context,
                &owner,
                DashPayProfile {
                    display_name: Some("Synced name".into()),
                    public_message: Some("New biography".into()),
                    bio: Some("New biography".into()),
                    avatar_url: Some("https://example.com/new.png".into()),
                    avatar_hash: None,
                    avatar_fingerprint: None,
                },
            )
            .await;
            editor.run();
            if !dirty {
                editor.get_by_label("Save social profile").click();
                editor.run();
                assert!(
                    editor.state().1.is_none(),
                    "synced fields must be a clean baseline"
                );
                type_draft(&mut editor);
            }
            editor.get_by_label("Save social profile").click();
            editor.run();
            let Some(BackendTask::DashPayTask(task)) = editor.state_mut().1.take() else {
                panic!("save dispatch")
            };
            let DashPayTask::UpdateProfile {
                display_name,
                bio,
                avatar_url,
                ..
            } = *task
            else {
                panic!("profile save")
            };
            let expected_name = if dirty {
                "Cached profile draft"
            } else {
                "Synced name draft"
            };
            assert_eq!(display_name.as_deref(), Some(expected_name));
            assert_eq!(bio.as_deref(), Some("New biography"));
            assert_eq!(avatar_url.as_deref(), Some("https://example.com/new.png"));
        }
    }

    #[test]
    fn hub_avatar_completions_only_retain_requested_urls() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut hub = IdentityHubScreen::new(&context);
        let url = "https://example.com/requested.png";
        let unrelated = "https://example.com/contact.png";
        hub.avatar_cache.ensure_requested(url);
        assert!(hub.is_waiting_for_avatar(url));
        assert!(!hub.is_waiting_for_avatar(unrelated));
        for current in [unrelated, url] {
            hub.display_backend_task_result(
                &BackendTaskContext::Other,
                BackendTaskSuccessResult::DashPayAvatar {
                    url: current.into(),
                    bytes: Some(vec![1, 2]),
                },
            );
        }
        assert!(
            hub.avatar_cache.fetched_bytes(unrelated).is_none(),
            "unrelated contact bytes must not be retained"
        );
        assert_eq!(hub.avatar_cache.fetched_bytes(url), Some(&[1, 2][..]));
        assert!(hub.avatar_cache.ensure_requested(url).is_none());
        assert!(!hub.is_waiting_for_avatar(url));
        hub.display_task_result(BackendTaskSuccessResult::DashPayAvatar {
            url: url.into(),
            bytes: Some(vec![3]),
        });
        assert_eq!(
            hub.avatar_cache.fetched_bytes(url),
            Some(&[1, 2][..]),
            "duplicate completion keeps the first result"
        );
        hub.avatar_cache.invalidate();
        hub.display_task_result(BackendTaskSuccessResult::DashPayAvatar {
            url: url.into(),
            bytes: Some(vec![4]),
        });
        assert!(
            hub.avatar_cache.fetched_bytes(url).is_none(),
            "obsolete completion stays discarded"
        );

        hub.avatar_cache.seed(url, vec![5]);
        hub.refresh_on_arrival();
        assert_eq!(hub.avatar_cache.fetched_bytes(url), Some(&[5][..]));
        assert!(hub.avatar_cache.ensure_requested(url).is_some());
        assert!(hub.is_waiting_for_avatar(url), "the renewal is awaited");
        hub.display_task_result(BackendTaskSuccessResult::DashPayAvatar {
            url: url.into(),
            bytes: Some(vec![6]),
        });
        assert_eq!(
            hub.avatar_cache.fetched_bytes(url),
            Some(&[6][..]),
            "the renewal replaces the kept image"
        );
    }

    async fn seed_cached_profile(context: &Arc<AppContext>) -> QualifiedIdentity {
        let id = seed_user_identity(context, 42);
        let identity = context.load_local_user_identities().unwrap().remove(0);
        let backend = context.wallet_backend().unwrap();
        let wallet =
            crate::model::wallet::Wallet::new_from_seed([42; 64], Network::Testnet, None, None)
                .unwrap();
        let (hash, _) = context
            .register_wallet(
                wallet,
                &[42; 64],
                crate::model::wallet::birth_height::WalletOrigin::Fresh,
            )
            .unwrap();
        backend
            .register_wallet_from_seed(&hash, &[42; 64], None)
            .await
            .unwrap();
        backend
            .ensure_identity_managed(&hash, &identity.identity, 0)
            .await
            .unwrap();
        backend
            .dashpay_set_profile(
                &id,
                Some(
                    platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile {
                        display_name: Some("Cached profile".into()),
                        avatar_url: Some("https://example.com/cached.png".into()),
                        bio: None,
                        public_message: None,
                        avatar_hash: None,
                        avatar_fingerprint: None,
                    },
                ),
            )
            .await
            .unwrap();
        identity
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_held_profile_primes_the_avatar_and_follows_the_synced_profile() {
        use platform_wallet::wallet::identity::types::dashpay::profile::DashPayProfile;
        let (_dir, context) = wired_test_context().await;
        let identity = seed_cached_profile(&context).await;
        let id = identity.identity.id();
        context
            .wallet_backend()
            .unwrap()
            .refresh_dashpay_snapshots()
            .await;
        let mut harness = egui_kittest::Harness::builder().build_ui_state(
            |ui, screen: &mut IdentityHubScreen| {
                screen.ui(ui);
            },
            IdentityHubScreen::new(&context),
        );
        // Avatar fetching stays pending in this offline harness, so its spinner keeps repainting.
        harness.run_steps(2);
        assert!(
            harness
                .state()
                .is_waiting_for_avatar("https://example.com/cached.png")
        );
        let fields = harness
            .state()
            .profiles
            .get(&identity)
            .and_then(Option::as_ref)
            .expect("the profile platform-wallet holds is available offline")
            .clone();
        assert_eq!(fields.avatar_url, "https://example.com/cached.png");
        sync_profile(
            &context,
            &id,
            DashPayProfile {
                display_name: Some("Saved".into()),
                avatar_url: None,
                bio: None,
                public_message: None,
                avatar_hash: None,
                avatar_fingerprint: None,
            },
        )
        .await;
        harness.run_steps(2);
        assert_eq!(
            harness
                .state()
                .profiles
                .get(&identity)
                .and_then(Option::as_ref)
                .map(|profile| profile.display_name.as_str()),
            Some("Saved")
        );
    }

    /// Give the identity-scoped caches state a reset would clear.
    fn warm_caches(screen: &mut IdentityHubScreen) {
        *screen.contacts_state.search_mut() = "warm".into();
        screen.dashpay_sync_due = false;
    }

    /// Whether the identity-scoped caches were reset since [`warm_caches`].
    fn caches_reset(screen: &mut IdentityHubScreen) -> bool {
        screen.contacts_state.search_mut().is_empty() && screen.dashpay_sync_due
    }

    /// The Hub's identity-scoped caches follow the *effective* identity, which
    /// is a fallback whenever no explicit selection is stored — the ordinary
    /// state for a single-identity account, where the user never touches the
    /// picker. Unloading that identity must therefore reset the caches even
    /// though `selected_identity_id()` names nothing, or Contacts keeps the
    /// removed identity's rows and its spent one-shot load guard while every
    /// action resolves under whichever identity became the fallback.
    ///
    /// Re-resolving the fallback here cannot answer the question: by the time
    /// this result lands the removed identity is gone from storage, so the
    /// resolver names its replacement. The Settings tab's retained identity is
    /// the surviving record of which identity the unload was started for.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unloading_a_fallback_selected_identity_resets_the_identity_caches() {
        let (_temp_dir, context) = wired_test_context().await;
        let fallback = seed_user_identity(&context, 1);
        let fallback_identity = context
            .get_local_qualified_identity(&fallback)
            .expect("read the seeded identity")
            .expect("identity present");
        let mut screen = IdentityHubScreen::new(&context);
        screen
            .settings_tab
            .select_identity_for_test(fallback_identity.clone());

        assert_eq!(
            context.selected_identity_id(),
            None,
            "precondition: the identity is effective by fallback, not by an explicit selection"
        );
        warm_caches(&mut screen);

        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            network: screen.app_context.network,
            identity_ids: vec![fallback],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            caches_reset(&mut screen),
            "the caches must be reset, or Contacts keeps the removed identity's rows and \
             never syncs the identity that replaced it"
        );
    }

    /// The frame ordering the previous fix missed. The unload is answered
    /// frames after dispatch, and `SettingsTab::ensure_selected` reconciles the
    /// tab's retained identity to the effective one on every frame in between —
    /// so a single render before the result lands leaves the explicit selection,
    /// the tab, and storage all naming the identity that replaced the removed
    /// one. Checking live pointers alone then finds nothing to reset and leaves
    /// contact rows, the one-shot load guard, pending tasks and confirmations,
    /// and the profile cache bound to an identity that no longer exists.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unload_answered_after_a_reconciling_frame_still_resets_the_caches() {
        let (_temp_dir, context) = wired_test_context().await;
        let unloaded = seed_user_identity(&context, 1);
        let replacement = seed_user_identity(&context, 2);
        let replacement_identity = context
            .get_local_qualified_identity(&replacement)
            .expect("read the seeded identity")
            .expect("identity present");
        let mut screen = IdentityHubScreen::new(&context);

        // The user confirms the unload: the screen dispatches it for `unloaded`.
        screen.capture_unload_dispatch(&AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::RemoveIdentity {
                identity_id: unloaded,
            },
        )));

        // A frame renders before the result arrives, and the tab reconciles
        // onto the identity that is now effective. Both live pointers now name
        // the replacement, or nothing.
        screen
            .settings_tab
            .select_identity_for_test(replacement_identity);
        assert_eq!(
            context.selected_identity_id(),
            None,
            "precondition: no explicit selection, so only the dispatch record names the target"
        );

        warm_caches(&mut screen);

        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            network: screen.app_context.network,
            identity_ids: vec![unloaded],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            caches_reset(&mut screen),
            "the caches must be reset even though no live pointer still names the removed \
             identity"
        );
    }

    /// Two unloads in flight at once. The confirmation blocks input only while
    /// it is open, so the user can confirm A, switch to B and confirm B before
    /// A's result lands — and A's result then arrives second, when nothing live
    /// names A any more. A single remembered id would have been overwritten by
    /// B, leaving A's caches bound to an identity that no longer exists.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_unload_dispatched_first_does_not_erase_the_first_ones_record() {
        let (_temp_dir, context) = wired_test_context().await;
        let first = seed_user_identity(&context, 1);
        let second = seed_user_identity(&context, 2);
        let survivor = seed_user_identity(&context, 3);
        let survivor_identity = context
            .get_local_qualified_identity(&survivor)
            .expect("read the seeded identity")
            .expect("identity present");
        let mut screen = IdentityHubScreen::new(&context);

        // Both confirmed before either result arrives.
        for identity_id in [first, second] {
            screen.capture_unload_dispatch(&AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RemoveIdentity { identity_id },
            )));
        }

        // Frames render in between, so every live pointer names the survivor.
        screen
            .settings_tab
            .select_identity_for_test(survivor_identity);

        // The second unload is answered first, and must not take the first
        // one's record with it.
        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            network: screen.app_context.network,
            identity_ids: vec![second],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
            cleanup_deferred: false,
        });
        assert!(
            screen.pending_unloads.contains(&first),
            "answering one unload must not discard the record of another still in flight"
        );

        // Now the first one's result lands, with nothing live naming it.
        warm_caches(&mut screen);

        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            network: screen.app_context.network,
            identity_ids: vec![first],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            caches_reset(&mut screen),
            "the first unload's caches must be reset when its own result arrives"
        );
        assert!(
            screen.pending_unloads.is_empty(),
            "each answered unload must retire its own record"
        );
    }

    /// A removal that fails before delisting sends no `RemovedIdentities`, so
    /// its record is never consumed. That entry is inert rather than harmless
    /// by assumption: the check is keyed on the identities a *result* removed,
    /// never on the pending set, so a stale entry can only ever match a later
    /// result removing that same identity — which is when the reset is wanted.
    /// It cannot cause a spurious reset for any other identity.
    ///
    /// Asserted rather than argued because the alternative — clearing on the
    /// error path — is not available: `display_message` carries a string and a
    /// severity, no identity, so the screen cannot know which of several
    /// in-flight unloads failed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_unloads_stale_record_never_resets_another_identitys_caches() {
        let (_temp_dir, context) = wired_test_context().await;
        let failed = seed_user_identity(&context, 1);
        let elsewhere = seed_user_identity(&context, 2);
        let mut screen = IdentityHubScreen::new(&context);

        // Dispatched, then the removal fails before delisting: no result ever
        // arrives for it, so its record stays.
        screen.capture_unload_dispatch(&AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::RemoveIdentity {
                identity_id: failed,
            },
        )));

        // Caches are warm and belong to the identity still in use.
        warm_caches(&mut screen);

        // An unrelated identity is removed — by another device, or by a screen
        // that is not this one. Nothing live names it, and this screen never
        // dispatched it.
        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            network: screen.app_context.network,
            identity_ids: vec![elsewhere],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            !caches_reset(&mut screen),
            "a removal naming no identity this screen cares about must not reset its caches"
        );
        assert!(
            screen.pending_unloads.contains(&failed),
            "and must not consume the failed unload's record, which belongs to another identity"
        );
    }

    /// A bare `WalletStorageNotReady` names no request, so it releases no guard.
    /// The migration gate now wraps a rejected DashPay contact action in
    /// `DashPayContactRequestActionFailed`, so a bare variant only reaches here
    /// for tasks that never claimed a guard. A blanket clear would re-enable a row
    /// whose paid action is genuinely still in flight.
    #[test]
    fn a_bare_storage_not_ready_releases_no_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(req(1)));
        assert!(state.begin_request(req(2)));

        release_request_guard_for_error(&mut state, &TaskError::WalletStorageNotReady);

        assert!(
            state.is_in_flight(&req(1)) && state.is_in_flight(&req(2)),
            "a bare storage-not-ready names no request and must leave every guard intact",
        );
    }

    /// The migration gate rejects a DashPay contact action with its request ID
    /// wrapped in `DashPayContactRequestActionFailed`, so the Hub releases only
    /// that request's guard. A different action genuinely in flight keeps its
    /// guard — the race the old blanket clear lost.
    #[test]
    fn a_gate_rejected_contact_action_releases_only_its_own_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(req(1))); // genuinely executing
        assert!(state.begin_request(req(2))); // about to be gate-rejected

        release_request_guard_for_error(
            &mut state,
            &TaskError::DashPayContactRequestActionFailed {
                request: req(2),
                source: Box::new(TaskError::WalletStorageNotReady),
            },
        );

        assert!(
            !state.is_in_flight(&req(2)),
            "the gate-rejected request's guard is released so its row un-sticks",
        );
        assert!(
            state.is_in_flight(&req(1)),
            "a different action still in flight must keep its guard through the gate rejection",
        );
    }

    /// Only a typed contact-action failure carries the request ID needed to
    /// release a guard. Every unrelated failure leaves all guards intact.
    #[test]
    fn an_unrelated_error_preserves_the_request_guards() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(req(1)));

        release_request_guard_for_error(&mut state, &TaskError::WalletLocked);

        assert!(
            state.is_in_flight(&req(1)),
            "only the request's own typed failure with its request ID releases a guard",
        );
    }

    #[test]
    fn unrelated_task_error_does_not_release_in_flight_paid_accept_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(req(2)));

        release_request_guard_for_error(&mut state, &TaskError::DocumentNotFound);

        assert!(
            state.is_in_flight(&req(2)),
            "a concurrent load error must not enable a second paid Accept"
        );
    }

    #[test]
    fn matching_request_error_releases_only_its_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(req(2)));
        assert!(state.begin_request(req(3)));
        let error = TaskError::DashPayContactRequestActionFailed {
            request: req(2),
            source: Box::new(TaskError::DocumentNotFound),
        };

        release_request_guard_for_error(&mut state, &error);

        assert!(!state.is_in_flight(&req(2)));
        assert!(state.is_in_flight(&req(3)));
    }

    fn contact_info_task(update: crate::model::dashpay::ContactInfoUpdate) -> DashPayTask {
        DashPayTask::UpdateContactInfo {
            identity: hub_identity(id(1)),
            contact_id: id(5),
            update,
        }
    }

    fn hub_identity(identifier: Identifier) -> QualifiedIdentity {
        QualifiedIdentity {
            identity: Identity::create_basic_identity(identifier, PlatformVersion::latest())
                .expect("basic identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    #[test]
    fn a_saved_note_and_an_unhide_report_differently() {
        use crate::model::dashpay::ContactInfoUpdate;

        let note = contact_info_task(ContactInfoUpdate::note(Some("Bao".into())));
        let unhide = contact_info_task(ContactInfoUpdate::visibility(false));
        assert!(is_note_save(&note));
        assert!(!is_note_save(&unhide));
        assert_eq!(
            contact_info_saved_message(&note),
            crate::ui::identity::contacts::NOTE_SAVED
        );
        assert_eq!(
            contact_info_saved_message(&unhide),
            "This contact is back in your list."
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_note_save_releases_only_its_own_guard() {
        let (_temp_dir, app_context) = wired_test_context().await;
        let mut screen = IdentityHubScreen::new(&app_context);
        let state = &mut screen.contacts_state;
        state.record_contacts_for_test(vec![crate::backend_task::dashpay::ContactData {
            identity_id: id(4),
            nickname: None,
            note: None,
            is_hidden: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }]);
        state.select_contact(&id(4));
        *state.detail_mut().note_mut(&id(4)).expect("draft") = "Bao".into();
        assert!(state.detail_mut().begin_save(id(3), id(4)).is_some());

        screen.handle_contact_info_error(&TaskError::WalletLocked);
        assert!(screen.contacts_state.detail().is_writing(&id(3), &id(4)));

        screen.handle_contact_info_error(&TaskError::DashPayContactInfoActionFailed {
            identity_id: id(3),
            contact_id: id(4),
            source: Box::new(TaskError::WalletLocked),
        });
        assert!(!screen.contacts_state.detail().is_writing(&id(3), &id(4)));
    }

    /// A note save for identity A that is still running when the user switches
    /// to B and back keeps its dispatch: its success settles it, and either
    /// kind of failure releases it, instead of leaving A's contact on Saving….
    async fn settle_after_an_identity_switch(outcome: &str) {
        use crate::model::dashpay::ContactInfoUpdate;
        let (owner, contact) = (id(3), id(4));
        let wrapped = TaskError::DashPayContactInfoActionFailed {
            identity_id: owner,
            contact_id: contact,
            source: Box::new(TaskError::WalletLocked),
        };
        {
            let (_temp_dir, app_context) = wired_test_context().await;
            let mut screen = IdentityHubScreen::new(&app_context);
            screen.contacts_state.record_contacts_for_test(vec![
                crate::backend_task::dashpay::ContactData {
                    identity_id: contact,
                    nickname: None,
                    note: None,
                    is_hidden: false,
                    account_reference: 0,
                    username: None,
                    display_name: None,
                    avatar_url: None,
                    bio: None,
                },
            ]);
            screen.contacts_state.select_contact(&contact);
            *screen
                .contacts_state
                .detail_mut()
                .note_mut(&contact)
                .expect("draft") = "Bao".into();
            assert!(
                screen
                    .contacts_state
                    .detail_mut()
                    .begin_save(owner, contact)
                    .is_some()
            );
            let action = AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
                DashPayTask::UpdateContactInfo {
                    identity: hub_identity(owner),
                    contact_id: contact,
                    update: ContactInfoUpdate::note(Some("Bao".into())),
                },
            )));
            screen.capture_contact_info_update(&action);
            let AppAction::BackendTaskWithContext {
                context: dispatched,
                ..
            } = screen.bind_contact_info_dispatch(action)
            else {
                panic!("the write is sent under its own dispatch");
            };

            // A → B → A while the write runs.
            screen.reset_contacts_for_identity_change();
            screen.reset_contacts_for_identity_change();

            match outcome {
                "success" => screen.display_backend_task_result(
                    &dispatched,
                    BackendTaskSuccessResult::DashPayContactInfoUpdated {
                        identity: owner,
                        contact_id: contact,
                        published: true,
                    },
                ),
                "wrapped failure" => {
                    screen.display_backend_task_error(&dispatched, &wrapped);
                    screen.display_task_error(&wrapped);
                }
                _ => {
                    screen.display_backend_task_error(&dispatched, &TaskError::WalletLocked);
                    screen.display_task_error(&TaskError::WalletLocked);
                }
            }

            assert!(
                !screen.contacts_state.detail().is_writing(&owner, &contact),
                "{outcome} after A → B → A must settle the write"
            );
            assert!(
                screen.contact_info_dispatches.is_empty()
                    && screen.pending_contact_info_tasks.is_empty(),
                "{outcome}: the dispatch record retires with its guard"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_succeeding_after_an_identity_switch_settles() {
        settle_after_an_identity_switch("success").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_failing_after_an_identity_switch_is_released() {
        settle_after_an_identity_switch("wrapped failure").await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_write_failing_bare_after_an_identity_switch_is_released() {
        settle_after_an_identity_switch("bare failure").await;
    }
}
