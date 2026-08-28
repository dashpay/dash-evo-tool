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
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::dashpay::UnreadableContactInfoPolicy;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel_with_breadcrumb;
use crate::ui::state::hub_selection::{HubSelection, HubView, effective_view};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Context};
use std::collections::{BTreeSet, HashMap, VecDeque};
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
    contact_info_overwrite_dialog: Option<(ConfirmationDialog, ContactInfoTaskKey)>,
    pending_contact_info_tasks: HashMap<ContactInfoTaskKey, DashPayTask>,
    pending_contact_info_confirmations: VecDeque<ContactInfoTaskKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ContactInfoTaskKey {
    Direct {
        identity_id: Identifier,
        contact_id: Identifier,
    },
    Request(Identifier),
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
            pending_unloads: BTreeSet::new(),
            contacts_state: super::contacts::ContactsState::default(),
            profile_cache: super::profile_cache::ProfileCache::default(),
            selection: HubSelection::default(),
            contact_info_overwrite_dialog: None,
            pending_contact_info_tasks: HashMap::new(),
            pending_contact_info_confirmations: VecDeque::new(),
        }
    }

    fn capture_contact_info_update(&mut self, action: &AppAction) {
        let AppAction::BackendTask(BackendTask::DashPayTask(task)) = action else {
            return;
        };
        if can_confirm_contact_info_overwrite(task)
            && let Some(key) = contact_info_task_key(task)
        {
            self.pending_contact_info_tasks
                .insert(key, task.as_ref().clone());
        }
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
        release_request_guards_for_abandoned_confirmations(
            &mut self.contacts_state,
            self.pending_contact_info_confirmations.iter().chain(
                self.contact_info_overwrite_dialog
                    .iter()
                    .map(|(_, key)| key),
            ),
        );
        self.contacts_state.reset_for_identity_change();
        self.pending_contact_info_tasks.clear();
        self.pending_contact_info_confirmations.clear();
        self.contact_info_overwrite_dialog = None;
    }

    fn queue_contact_info_confirmation(&mut self, key: ContactInfoTaskKey) {
        let already_showing = self
            .contact_info_overwrite_dialog
            .as_ref()
            .is_some_and(|(_, shown_key)| *shown_key == key);
        if already_showing || self.pending_contact_info_confirmations.contains(&key) {
            return;
        }

        self.pending_contact_info_confirmations.push_back(key);
    }

    fn prepare_contact_info_dialog(&mut self) {
        if self.contact_info_overwrite_dialog.is_some() {
            return;
        }
        while let Some(key) = self.pending_contact_info_confirmations.pop_front() {
            if let Some(task) = self.pending_contact_info_tasks.get(&key) {
                let (title, message) = contact_info_confirmation_copy(task);
                self.contact_info_overwrite_dialog = Some((
                    ConfirmationDialog::new(title, message)
                        .confirm_text(Some("Replace saved details"))
                        .danger_mode(true),
                    key,
                ));
                break;
            }
        }
    }

    /// Reset all identity-scoped state after the owning application context
    /// changes. Abandoned confirmations release their guards; live paid actions
    /// remain guarded, and stale guards are pruned during the contacts reset.
    pub(crate) fn reset_for_context_change(&mut self) {
        self.reset_contacts_for_identity_change();
        self.profile_cache.reset();
        self.selection.clear_picker_override();
        self.selection.clear_searches();
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

    /// Whether an identity-scoped backend result still belongs to the identity
    /// on screen. See [`applies_to_selected_identity`].
    fn result_is_for_selected_identity(&self, result_identity: &Identifier) -> bool {
        let selected = self
            .app_context
            .resolve_selected_identity()
            .map(|identity| identity.identity.id());
        applies_to_selected_identity(selected, result_identity)
    }

    /// Retire a request row the backend just resolved (accepted, declined, or
    /// cancelled), confirm it to the user, and re-arm the Contacts load so the
    /// authoritative lists replace the local edit.
    fn resolve_request(&mut self, request_id: &Identifier, confirmation: &str) {
        self.pending_contact_info_tasks
            .remove(&ContactInfoTaskKey::Request(*request_id));
        self.contacts_state.remove_request(request_id);
        self.contacts_state.invalidate();
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
            BackendTaskSuccessResult::DashPayContactRequestAccepted(request_id) => {
                self.resolve_request(request_id, "Contact request accepted.");
            }
            BackendTaskSuccessResult::DashPayContactRequestRejected(request_id) => {
                self.resolve_request(request_id, "Contact request declined.");
            }
            BackendTaskSuccessResult::DashPayContactRequestCancelled(request_id) => {
                self.resolve_request(request_id, "Contact request cancelled.");
            }
            BackendTaskSuccessResult::DashPayContactAlreadyEstablished { request_id, .. } => {
                self.pending_contact_info_tasks
                    .remove(&ContactInfoTaskKey::Request(*request_id));
                self.contacts_state.remove_request(request_id);
                self.contacts_state.invalidate();
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
        if let Some(key) = contact_info_read_error_key(error)
            && self.pending_contact_info_tasks.contains_key(&key)
        {
            self.contacts_state.invalidate();
            self.queue_contact_info_confirmation(key);
            return true;
        }
        if let Some(key) = contact_info_error_key(error) {
            self.pending_contact_info_tasks.remove(&key);
        }
        release_request_guard_for_error(&mut self.contacts_state, error);
        true
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
                self.reset_contacts_for_identity_change();
                self.profile_cache.reset();
                AppAction::None
            }
            BreadcrumbEffect::SelectIdentity(id) => {
                self.app_context.set_selected_identity(Some(id));
                self.selection.clear_picker_override();
                self.reset_contacts_for_identity_change();
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
        self.capture_contact_info_update(&action);
        self.capture_unload_dispatch(&action);

        // A picker card click sets the active identity and routes to Home.
        if let Some(id_str) = picked_identity
            && let Ok(id) = Identifier::from_string_unknown_encoding(&id_str)
        {
            self.app_context.set_selected_identity(Some(id));
            self.selection.clear_picker_override();
            self.reset_contacts_for_identity_change();
            self.profile_cache.reset();
        }

        action |= self.apply_breadcrumb_effect(breadcrumb_effect);

        // Dispatch any profile load a tab requested this frame (single load
        // in flight; the local profile cache was removed in the platform-wallet
        // migration, so profiles resolve asynchronously via the backend).
        action |= self.profile_cache.dispatch_pending();

        self.prepare_contact_info_dialog();
        if let Some((dialog, key)) = &mut self.contact_info_overwrite_dialog {
            let key = *key;
            match dialog.show(ui).inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) => {
                    if let Some(task) = self
                        .pending_contact_info_tasks
                        .get(&key)
                        .cloned()
                        .and_then(overwrite_unreadable_task)
                    {
                        self.pending_contact_info_tasks.insert(key, task.clone());
                        action = AppAction::BackendTask(BackendTask::DashPayTask(Box::new(task)));
                    }
                    self.contact_info_overwrite_dialog = None;
                }
                Some(ConfirmationStatus::Canceled) => {
                    if let ContactInfoTaskKey::Request(request_id) = key {
                        self.contacts_state.release_request(&request_id);
                    }
                    self.pending_contact_info_tasks.remove(&key);
                    self.contacts_state.invalidate();
                    self.contact_info_overwrite_dialog = None;
                }
                None => {}
            }
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // AppState sets the global banner centrally; the hub itself owns no
        // in-flight task banners. Sub-tab content overrides its own lifecycle.
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // Feed an async DashPay profile load back into the cache the tabs read.
        self.profile_cache.record_result(&result);

        if self.handle_contact_request_result(&result) {
            return;
        }

        match &result {
            // A confirmed profile-save success: commit the edit baseline on the
            // Settings tab so the Save button re-enables only after the next
            // edit. Guard by identity ID to reject stale results.
            BackendTaskSuccessResult::DashPayProfileUpdated(saved_id) => {
                handle_profile_updated(
                    &mut self.settings_tab,
                    &mut self.profile_cache,
                    self.app_context.egui_ctx(),
                    *saved_id,
                );
            }
            // Populate the Received/Sent request caches so the Contacts tab
            // can render real RequestCard rows instead of hardcoded empties.
            // The result arrives from LoadContactRequests,
            // dispatched alongside LoadContacts in contacts::render_populated.
            BackendTaskSuccessResult::DashPayContactRequests {
                identity,
                incoming,
                outgoing,
            } => {
                if self.result_is_for_selected_identity(identity) {
                    self.contacts_state
                        .record_requests(incoming.clone(), outgoing.clone());
                }
            }
            // The established-contact list behind the Contacts tab's active
            // section and its search box.
            BackendTaskSuccessResult::DashPayContactsWithInfo { identity, contacts } => {
                if self.result_is_for_selected_identity(identity) {
                    self.contacts_state.record_contacts(contacts.clone());
                }
            }
            // A confirmed contactInfo write — on this tab that is an unhide.
            // Re-arm the load so the restored contact comes back from the
            // authoritative list, not just the optimistic local move.
            BackendTaskSuccessResult::DashPayContactInfoUpdated {
                identity,
                contact_id,
            } => {
                let key = ContactInfoTaskKey::Direct {
                    identity_id: *identity,
                    contact_id: *contact_id,
                };
                if self.pending_contact_info_tasks.remove(&key).is_some() {
                    self.contacts_state.invalidate();
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "This contact is back in your list.",
                        MessageType::Success,
                    );
                }
            }
            // The Settings tab's unload lands here. The tab re-resolves its own
            // selection next frame, so this only has to report the outcome —
            // including the case where the identity went but its voter twin
            // stayed, which the user would otherwise read as a failed removal.
            BackendTaskSuccessResult::RemovedIdentities {
                identity_ids,
                associated_cleanup_failed,
                cleanup_deferred,
            } => {
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
                    self.profile_cache.reset();
                }
                // Only the identities this result answers for; another unload
                // still in flight keeps its own record.
                self.pending_unloads
                    .retain(|pending| !identity_ids.contains(pending));
                let (message, message_type) = crate::ui::identities::removed_identities_banner(
                    *associated_cleanup_failed,
                    *cleanup_deferred,
                );
                let handle =
                    MessageBanner::set_global(self.app_context.egui_ctx(), message, message_type);
                if message_type == MessageType::Warning {
                    handle.disable_auto_dismiss();
                }
            }
            _ => {}
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if let Some(identity_id) = context.dashpay_profile_update_identity() {
            self.settings_tab
                .clear_pending_save_for_identity(&identity_id);
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if self.handle_contact_request_error(error) {
            return matches!(
                contact_info_read_error_key(error),
                Some(key) if self.pending_contact_info_tasks.contains_key(&key)
            );
        }
        if let Some(key) = contact_info_read_error_key(error)
            && self.pending_contact_info_tasks.contains_key(&key)
        {
            self.contacts_state.invalidate();
            self.queue_contact_info_confirmation(key);
            return true;
        }

        if let Some(key) = contact_info_error_key(error) {
            self.pending_contact_info_tasks.remove(&key);
        }
        release_request_guard_for_error(&mut self.contacts_state, error);

        // Let AppState render the default error banner — the hub has no
        // other special-case error handling of its own yet.
        false
    }
}

/// Release the request-card guard a failed contact action names. Both a task-level
/// failure and a migration-gate refusal arrive as
/// [`TaskError::DashPayContactRequestActionFailed`], carrying the request ID, so
/// only that request's guard is released; every other guard stays protected until
/// its own typed outcome arrives — a paid action still in flight must never have
/// its row re-enabled by an unrelated failure.
fn release_request_guard_for_error(state: &mut super::contacts::ContactsState, error: &TaskError) {
    if let TaskError::DashPayContactRequestActionFailed { request_id, .. } = error {
        state.release_request(request_id);
    }
}

fn release_request_guards_for_abandoned_confirmations<'a>(
    state: &mut super::contacts::ContactsState,
    keys: impl IntoIterator<Item = &'a ContactInfoTaskKey>,
) {
    for key in keys {
        if let ContactInfoTaskKey::Request(request_id) = key {
            state.release_request(request_id);
        }
    }
}

fn can_confirm_contact_info_overwrite(task: &DashPayTask) -> bool {
    match task {
        DashPayTask::UpdateContactInfo { update, .. } => {
            update.unreadable == UnreadableContactInfoPolicy::Abort
        }
        DashPayTask::RejectContactRequest { unreadable, .. }
        | DashPayTask::CancelContactRequest { unreadable, .. } => {
            *unreadable == UnreadableContactInfoPolicy::Abort
        }
        _ => false,
    }
}

/// Title and body for the overwrite confirmation, one complete message per
/// action so each stays a single translatable sentence group. Only the contact
/// case names an identifier: a request ID is not something the user has seen,
/// and the details at risk belong to the contact, not to the request.
fn contact_info_confirmation_copy(task: &DashPayTask) -> (&'static str, String) {
    use crate::ui::identity::identity_pill::shorten_id;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;

    match task {
        DashPayTask::UpdateContactInfo { contact_id, .. } => {
            let contact = shorten_id(&contact_id.to_string(Encoding::Base58));
            (
                "Unhide contact and replace saved details?",
                format!(
                    "The saved details for contact {contact} cannot be read. Unhiding this contact will clear its saved nickname, note, and accepted-account settings. You cannot undo this change."
                ),
            )
        }
        DashPayTask::RejectContactRequest { .. } => (
            "Decline request and replace saved details?",
            "The saved details for this contact cannot be read. Declining the request will clear the contact's saved nickname, note, and accepted-account settings. You cannot undo this change.".to_string(),
        ),
        DashPayTask::CancelContactRequest { .. } => (
            "Cancel request and replace saved details?",
            "The saved details for this contact cannot be read. Withdrawing the request will clear the contact's saved nickname, note, and accepted-account settings. You cannot undo this change.".to_string(),
        ),
        _ => (
            "Replace saved contact details?",
            "The saved details for this contact cannot be read. Continuing will clear the contact's saved nickname, note, and accepted-account settings. You cannot undo this change.".to_string(),
        ),
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
        DashPayTask::RejectContactRequest { request_id, .. }
        | DashPayTask::CancelContactRequest { request_id, .. } => {
            Some(ContactInfoTaskKey::Request(*request_id))
        }
        _ => None,
    }
}

fn overwrite_unreadable_task(task: DashPayTask) -> Option<DashPayTask> {
    match task {
        DashPayTask::UpdateContactInfo {
            identity,
            contact_id,
            update,
        } => Some(DashPayTask::UpdateContactInfo {
            identity,
            contact_id,
            update: update.overwrite_unreadable(),
        }),
        DashPayTask::RejectContactRequest {
            identity,
            request_id,
            ..
        } => Some(DashPayTask::RejectContactRequest {
            identity,
            request_id,
            unreadable: UnreadableContactInfoPolicy::Overwrite,
        }),
        DashPayTask::CancelContactRequest {
            identity,
            request_id,
            ..
        } => Some(DashPayTask::CancelContactRequest {
            identity,
            request_id,
            unreadable: UnreadableContactInfoPolicy::Overwrite,
        }),
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
        TaskError::DashPayContactRequestActionFailed { request_id, .. } => {
            Some(ContactInfoTaskKey::Request(*request_id))
        }
        _ => None,
    }
}

fn contact_info_read_error_key(error: &TaskError) -> Option<ContactInfoTaskKey> {
    match error {
        TaskError::DashPayContactInfoActionFailed { source, .. }
        | TaskError::DashPayContactRequestActionFailed { source, .. }
            if matches!(source.as_ref(), TaskError::DashPayContactInfoRead { .. }) =>
        {
            contact_info_error_key(error)
        }
        _ => None,
    }
}

/// Whether a backend result loaded for `result_identity` may still be applied
/// while `selected` is the identity on screen.
///
/// Switching identity cannot cancel a load already in flight, so a result for
/// the previous identity can land after the switch. Applying it would paint one
/// identity's contacts and requests under another's name — and hand the user
/// rows that act under the wrong key.
fn applies_to_selected_identity(
    selected: Option<Identifier>,
    result_identity: &Identifier,
) -> bool {
    selected.as_ref() == Some(result_identity)
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

fn handle_profile_updated(
    settings: &mut SettingsTab,
    profiles: &mut super::profile_cache::ProfileCache,
    ctx: &egui::Context,
    saved_id: Identifier,
) {
    let matches = settings
        .selected_identity()
        .is_some_and(|identity| identity.identity.id() == saved_id);
    if matches && let Some(fields) = settings.on_profile_saved() {
        profiles.record_saved(saved_id, fields);
        MessageBanner::set_global(
            ctx,
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
    use dash_sdk::dpp::dashcore::Network;
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

    #[test]
    fn stale_profile_success_does_not_show_confirmation() {
        let ctx = egui::Context::default();
        let mut settings = SettingsTab::new();
        let mut profiles = crate::ui::identity::profile_cache::ProfileCache::default();

        handle_profile_updated(&mut settings, &mut profiles, &ctx, id(1));

        assert!(
            !MessageBanner::has_global(&ctx),
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn contact_results_follow_fallback_and_explicit_identity_selection() {
        let (_temp_dir, context) = wired_test_context().await;
        let fallback_identity = seed_user_identity(&context, 1);
        let screen = IdentityHubScreen::new(&context);

        assert_eq!(
            context.selected_identity_id(),
            None,
            "the single-identity path must not require an explicit picker selection"
        );
        assert!(
            screen.result_is_for_selected_identity(&fallback_identity),
            "a contact result for the fallback-resolved identity must populate the Contacts tab"
        );

        let explicitly_selected = seed_user_identity(&context, 2);
        context.set_selected_identity(Some(explicitly_selected));

        assert!(
            !screen.result_is_for_selected_identity(&fallback_identity),
            "a late result for the previous fallback identity must be rejected after a switch"
        );
        assert!(
            screen.result_is_for_selected_identity(&explicitly_selected),
            "a result for the explicitly selected identity must apply after a switch"
        );
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
        assert!(
            screen.contacts_state.claim_load(),
            "precondition: the one-shot load guard starts unclaimed"
        );
        screen.profile_cache.record_saved(
            fallback,
            crate::ui::identity::profile_cache::ProfileFields {
                display_name: "Alicia".into(),
                bio: String::new(),
                avatar_url: String::new(),
            },
        );

        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: vec![fallback],
            associated_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            screen.contacts_state.claim_load(),
            "the load guard must be re-armed, or Contacts renders the removed identity's \
             rows and never re-fetches for the identity that replaced it"
        );
        assert!(
            screen
                .profile_cache
                .get_or_request(&fallback_identity)
                .is_none(),
            "the removed identity's cached profile must be dropped"
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

        assert!(screen.contacts_state.claim_load());
        screen.profile_cache.record_saved(
            unloaded,
            crate::ui::identity::profile_cache::ProfileFields {
                display_name: "Alicia".into(),
                bio: String::new(),
                avatar_url: String::new(),
            },
        );

        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: vec![unloaded],
            associated_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            screen.contacts_state.claim_load(),
            "the load guard must be re-armed even though no live pointer still names the \
             removed identity"
        );
        let unloaded_identity = QualifiedIdentity {
            identity: Identity::create_basic_identity(unloaded, PlatformVersion::latest())
                .expect("identity"),
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
        assert!(
            screen
                .profile_cache
                .get_or_request(&unloaded_identity)
                .is_none(),
            "the removed identity's cached profile must be dropped"
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
            identity_ids: vec![second],
            associated_cleanup_failed: false,
            cleanup_deferred: false,
        });
        assert!(
            screen.pending_unloads.contains(&first),
            "answering one unload must not discard the record of another still in flight"
        );

        // Now the first one's result lands, with nothing live naming it.
        assert!(screen.contacts_state.claim_load());
        screen.profile_cache.record_saved(
            first,
            crate::ui::identity::profile_cache::ProfileFields {
                display_name: "Alicia".into(),
                bio: String::new(),
                avatar_url: String::new(),
            },
        );

        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: vec![first],
            associated_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            screen.contacts_state.claim_load(),
            "the first unload's caches must be reset when its own result arrives"
        );
        let first_identity = QualifiedIdentity {
            identity: Identity::create_basic_identity(first, PlatformVersion::latest())
                .expect("identity"),
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
        assert!(
            screen
                .profile_cache
                .get_or_request(&first_identity)
                .is_none(),
            "the first unloaded identity's cached profile must be dropped"
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
        assert!(screen.contacts_state.claim_load());

        // An unrelated identity is removed — by another device, or by a screen
        // that is not this one. Nothing live names it, and this screen never
        // dispatched it.
        screen.display_task_result(BackendTaskSuccessResult::RemovedIdentities {
            identity_ids: vec![elsewhere],
            associated_cleanup_failed: false,
            cleanup_deferred: false,
        });

        assert!(
            !screen.contacts_state.claim_load(),
            "a removal naming no identity this screen cares about must not reset its caches"
        );
        assert!(
            screen.pending_unloads.contains(&failed),
            "and must not consume the failed unload's record, which belongs to another identity"
        );
    }

    #[test]
    fn a_result_for_the_selected_identity_applies() {
        assert!(applies_to_selected_identity(Some(id(1)), &id(1)));
    }

    #[test]
    fn a_result_for_a_previously_selected_identity_is_discarded() {
        // The load was dispatched for identity A; the user switched to B before
        // it returned. A's contacts must never appear under B.
        assert!(!applies_to_selected_identity(Some(id(2)), &id(1)));
    }

    #[test]
    fn a_result_arriving_with_no_identity_selected_is_discarded() {
        assert!(!applies_to_selected_identity(None, &id(1)));
    }

    /// A bare `WalletStorageNotReady` names no request, so it releases no guard.
    /// The migration gate now wraps a rejected DashPay contact action in
    /// `DashPayContactRequestActionFailed`, so a bare variant only reaches here
    /// for tasks that never claimed a guard. A blanket clear would re-enable a row
    /// whose paid action is genuinely still in flight.
    #[test]
    fn a_bare_storage_not_ready_releases_no_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(1)));
        assert!(state.begin_request(id(2)));

        release_request_guard_for_error(&mut state, &TaskError::WalletStorageNotReady);

        assert!(
            state.is_in_flight(&id(1)) && state.is_in_flight(&id(2)),
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
        assert!(state.begin_request(id(1))); // genuinely executing
        assert!(state.begin_request(id(2))); // about to be gate-rejected

        release_request_guard_for_error(
            &mut state,
            &TaskError::DashPayContactRequestActionFailed {
                request_id: id(2),
                source: Box::new(TaskError::WalletStorageNotReady),
            },
        );

        assert!(
            !state.is_in_flight(&id(2)),
            "the gate-rejected request's guard is released so its row un-sticks",
        );
        assert!(
            state.is_in_flight(&id(1)),
            "a different action still in flight must keep its guard through the gate rejection",
        );
    }

    /// Only a typed contact-action failure carries the request ID needed to
    /// release a guard. Every unrelated failure leaves all guards intact.
    #[test]
    fn an_unrelated_error_preserves_the_request_guards() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(1)));

        release_request_guard_for_error(&mut state, &TaskError::WalletLocked);

        assert!(
            state.is_in_flight(&id(1)),
            "only the request's own typed failure with its request ID releases a guard",
        );
    }

    #[test]
    fn unrelated_task_error_does_not_release_in_flight_paid_accept_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(2)));

        release_request_guard_for_error(&mut state, &TaskError::DocumentNotFound);

        assert!(
            state.is_in_flight(&id(2)),
            "a concurrent load error must not enable a second paid Accept"
        );
    }

    #[test]
    fn matching_request_error_releases_only_its_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(2)));
        assert!(state.begin_request(id(3)));
        let error = TaskError::DashPayContactRequestActionFailed {
            request_id: id(2),
            source: Box::new(TaskError::DocumentNotFound),
        };

        release_request_guard_for_error(&mut state, &error);

        assert!(!state.is_in_flight(&id(2)));
        assert!(state.is_in_flight(&id(3)));
    }

    #[test]
    fn abandoning_confirmations_releases_only_their_request_guards() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(1)));
        assert!(state.begin_request(id(2)));
        let keys = [
            ContactInfoTaskKey::Request(id(2)),
            ContactInfoTaskKey::Direct {
                identity_id: id(3),
                contact_id: id(4),
            },
        ];

        release_request_guards_for_abandoned_confirmations(&mut state, &keys);

        assert!(state.is_in_flight(&id(1)));
        assert!(
            !state.is_in_flight(&id(2)),
            "a discarded confirmation has no running task left to protect"
        );
    }

    #[test]
    fn concurrent_contact_info_errors_keep_distinct_retry_keys() {
        use crate::backend_task::error::ContactInfoReadError;

        let request_error = TaskError::DashPayContactRequestActionFailed {
            request_id: id(2),
            source: Box::new(TaskError::DashPayContactInfoRead {
                source: ContactInfoReadError::DeserializeFailed,
            }),
        };
        let direct_error = TaskError::DashPayContactInfoActionFailed {
            identity_id: id(3),
            contact_id: id(4),
            source: Box::new(TaskError::DashPayContactInfoRead {
                source: ContactInfoReadError::DeserializeFailed,
            }),
        };

        assert_eq!(
            contact_info_read_error_key(&request_error),
            Some(ContactInfoTaskKey::Request(id(2)))
        );
        assert_eq!(
            contact_info_read_error_key(&direct_error),
            Some(ContactInfoTaskKey::Direct {
                identity_id: id(3),
                contact_id: id(4),
            })
        );
    }
}
