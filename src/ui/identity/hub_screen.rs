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
use crate::model::dashpay::{ContactInfoField, UnreadableContactInfoPolicy};
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel_with_breadcrumb;
use crate::ui::state::hub_selection::{HubSelection, HubView, effective_view};
use crate::ui::state::identity_tokens::IdentityTokensState;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dpp::dashcore::Network;
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
    /// Tokens-tab state: the selected identity's token rows, the balance
    /// refresh it waits on, and the list controls. Also feeds the tab badge.
    tokens_state: IdentityTokensState,
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
    /// Contacts-tab state. Owned here to debounce
    /// [`crate::backend_task::dashpay::DashPayTask::LoadContacts`] to a
    /// single dispatch per tab entry, instead of firing every frame.
    contacts_state: super::contacts::ContactsState,
    /// DashPay profiles, loaded asynchronously (the local profile cache was
    /// removed in the platform-wallet migration). Read by the Home, Contacts,
    /// and Settings tabs; loads are dispatched after rendering each frame.
    profile_cache: super::profile_cache::ProfileCache,
    avatar_cache: crate::ui::state::AvatarCache,
    pending_avatars: Vec<BackendTask>,
    /// Breadcrumb-switcher view state (picker override + dropdown search
    /// buffers). The active identity itself is app-scoped on `AppContext`.
    selection: HubSelection,
    contact_info_overwrite_dialog: Option<(ConfirmationDialog, ContactInfoTaskKey)>,
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
            tokens_state: IdentityTokensState::default(),
            usernames_refreshed_at: None,
            usernames_arriving: true,
            pending_unloads: BTreeSet::new(),
            contacts_state: super::contacts::ContactsState::default(),
            profile_cache: super::profile_cache::ProfileCache::default(),
            avatar_cache: crate::ui::state::AvatarCache::default(),
            pending_avatars: Vec::new(),
            selection: HubSelection::default(),
            contact_info_overwrite_dialog: None,
            pending_contact_info_tasks: HashMap::new(),
            contact_info_dispatches: HashMap::new(),
            foreign_contact_info_error: false,
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
        release_guards_for_abandoned_confirmations(
            &mut self.contacts_state,
            self.app_context.network,
            self.pending_contact_info_confirmations.iter().chain(
                self.contact_info_overwrite_dialog
                    .iter()
                    .map(|(_, key)| key),
            ),
        );
        self.home_state = HomeState::default();
        self.tokens_state.reset_for_identity_change();
        self.contacts_state.reset_for_identity_change();
        // A write still running keeps its guard across the change, so it keeps
        // the records its outcome is matched against too.
        let detail = self.contacts_state.detail();
        let live = |key: &ContactInfoTaskKey| {
            matches!(key, ContactInfoTaskKey::Direct { identity_id, contact_id }
                if detail.is_writing(identity_id, contact_id))
        };
        self.pending_contact_info_tasks.retain(|key, _| live(key));
        self.contact_info_dispatches.retain(|key, _| live(key));
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

    /// Clear the stale load-error banner so the next `landing()` call can try
    /// again cleanly, and re-arm the per-tab loads. With `keep_shown`, the
    /// profile and contacts stay on screen until their reload lands.
    fn reload(&mut self, keep_shown: bool) {
        self.load_error_banner.take_and_clear();
        if keep_shown {
            self.contacts_state.invalidate();
            self.contacts_state.search_mut().clear();
            self.profile_cache.reload_kept();
            self.avatar_cache.reload_kept();
        } else {
            self.contacts_state.reset();
            self.profile_cache.reset();
            self.avatar_cache.invalidate();
        }
        self.home_state.refresh_payments();
        self.tokens_state.invalidate_rows();
        self.selection.clear_searches();
        self.pending_avatars.clear();
    }

    /// Reset all identity-scoped state after the owning application context
    /// changes. Abandoned confirmations release their guards; live paid actions
    /// remain guarded, and stale guards are pruned during the contacts reset.
    pub(crate) fn reset_for_context_change(&mut self) {
        self.reset_contacts_for_identity_change();
        self.settings_tab = SettingsTab::new();
        self.tokens_state = IdentityTokensState::default();
        self.profile_cache.reset();
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
            self.forget_contact_info_task(&key);
        }
        release_request_guard_for_error(&mut self.contacts_state, error);
        true
    }

    /// Settle a failed direct contact details write: ask before overwriting
    /// unreadable details, otherwise drop the pending write and release its
    /// guard. Returns whether the hub took over telling the user.
    pub(crate) fn handle_contact_info_error(&mut self, error: &TaskError) -> bool {
        // Another screen's write failed; the hub's own write is still running.
        if std::mem::take(&mut self.foreign_contact_info_error)
            || !matches!(error, TaskError::DashPayContactInfoActionFailed { .. })
        {
            return false;
        }
        if let Some(key) = contact_info_read_error_key(error)
            && self.pending_contact_info_tasks.contains_key(&key)
        {
            self.contacts_state.invalidate();
            self.queue_contact_info_confirmation(key);
            return true;
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
    fn observe_backend_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        let needs_completion = match result {
            // Another screen's contact details write is newer than any note
            // the hub saved, so the hub retires that note and reloads.
            BackendTaskSuccessResult::DashPayProfileUpdated(_)
            | BackendTaskSuccessResult::DashPayContactsWithInfo { .. }
            | BackendTaskSuccessResult::DashPayContactRequests { .. }
            | BackendTaskSuccessResult::DashPayContactInfoUpdated { .. } => true,
            BackendTaskSuccessResult::DashPayAvatar { url, .. } => self.is_waiting_for_avatar(url),
            _ => false,
        };
        if needs_completion {
            self.display_backend_task_result(context, result.clone());
        } else {
            self.handle_contact_request_result(result);
        }
    }

    fn observe_backend_error(&mut self, _context: &BackendTaskContext, error: &TaskError) {
        self.handle_contact_request_error(error);
    }
    fn cached_data_changed(&mut self, update: crate::backend_task::CacheUpdate) {
        if update.network == self.app_context.network
            && update.domain == crate::backend_task::CacheDomain::All
        {
            self.load_error_banner.take_and_clear();
            self.profile_cache.reload_kept();
            self.contacts_state.invalidate();
        }
        if update.network == self.app_context.network
            && matches!(
                update.domain,
                crate::backend_task::CacheDomain::ContactPayments
                    | crate::backend_task::CacheDomain::All
            )
        {
            self.home_state.refresh_payments();
        }
        if update.network == self.app_context.network {
            self.tokens_state.cached_data_changed(update.domain);
        }
        // Identity selection and balances are read from AppContext each frame.
    }
    fn refresh(&mut self) {
        self.reload(false);
    }

    /// Returning keeps the profile and contacts already shown and reloads them
    /// behind; lists of another identity are dropped as the hub renders.
    fn refresh_on_arrival(&mut self) {
        self.reload(true);
        self.usernames_arriving = true;
        self.tokens_state.arrive();
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
        for identity in &frame_identities {
            self.profile_cache.seed_cached(&self.app_context, identity);
        }
        // A profile load sent before the masternode list synced (right after
        // start-up) failed; ask for it again now that the network is ready.
        if self.app_context.connection_status().masternodes_ready() {
            self.profile_cache.retry_after_sync();
            self.contacts_state
                .retry_load_if_due(std::time::Instant::now());
        }
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
                    &mut self.profile_cache,
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
                                &mut self.profile_cache,
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
                        let pending_claims = match &selected {
                            Some(identity) => self
                                .tokens_state
                                .badge_pending_claims(&self.app_context, identity),
                            None => 0,
                        };
                        let tab_response = IdentityHubTabBar::new(self.selected_tab)
                            .with_badge(
                                IdentityHubTab::Tokens,
                                pending_claims,
                                super::token_copy::pending_claims_text(pending_claims),
                            )
                            .show(ui);
                        if let Some(clicked) = tab_response.clicked() {
                            if clicked != self.selected_tab {
                                // Tab-entry transition: reset per-tab load guards
                                // so the incoming tab re-dispatches once.
                                self.contacts_state.reset();
                            }
                            self.selected_tab = clicked;
                        }
                        ui.add_space(16.0);
                        rendered_tab = Some(self.selected_tab);
                        if self.selected_tab != IdentityHubTab::Tokens {
                            // A filter still being typed when the tab is left
                            // is written now rather than lost.
                            self.tokens_state.save_prefs(&self.app_context);
                        }
                        let mut tab_action = match self.selected_tab {
                            IdentityHubTab::Home => {
                                let (home_action, outcome) = match &selected {
                                    Some(identity) => super::home::render(
                                        ui,
                                        &self.app_context,
                                        identity,
                                        &mut self.home_state,
                                        &mut self.profile_cache,
                                        &mut self.contacts_state,
                                        &mut self.tokens_state,
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
                                        self.contacts_state.invalidate();
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
                                &mut self.profile_cache,
                                &mut self.avatar_cache,
                                &mut self.pending_avatars,
                            ),
                            IdentityHubTab::Tokens => super::tokens_tab::render(
                                ui,
                                &self.app_context,
                                selected.as_ref(),
                                &mut self.tokens_state,
                                &mut self.home_state,
                            ),
                            IdentityHubTab::Activity => {
                                super::activity::render(ui, &self.app_context)
                            }
                            IdentityHubTab::Settings => self.settings_tab.render(
                                ui,
                                &self.app_context,
                                selected.as_ref(),
                                &mut self.profile_cache,
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
            self.profile_cache.reset();
        }

        action |= self.apply_breadcrumb_effect(breadcrumb_effect);
        // The top-bar "Add" menu and the picker's add card emit
        // `AppAction::Custom` commands; route them like the identity pill's
        // add entries.
        if let Some(effect) = breadcrumb_switcher::add_identity_command_effect(&action) {
            action = self.apply_breadcrumb_effect(effect);
        }

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
                        action = self.bind_contact_info_dispatch(AppAction::BackendTask(
                            BackendTask::DashPayTask(Box::new(task)),
                        ));
                    }
                    self.contact_info_overwrite_dialog = None;
                }
                Some(ConfirmationStatus::Canceled) => {
                    release_guards_for_abandoned_confirmations(
                        &mut self.contacts_state,
                        self.app_context.network,
                        [&key],
                    );
                    self.forget_contact_info_task(&key);
                    self.contacts_state.invalidate();
                    self.contact_info_overwrite_dialog = None;
                }
                None => {}
            }
        }

        // Every part of the frame above has had its say; this is where the
        // action that leaves the frame is settled. A data load claims its one
        // dispatch slot when it is asked for, so it is asked for only here and
        // only when nothing produced an action: a click anywhere on the screen
        // keeps its frame, and a claimed load is the action returned — nothing
        // below replaces an action.
        if matches!(action, AppAction::None) {
            action = match rendered_tab {
                // Home's Tokens card has no balances until the arrival refresh
                // registers the identity with the balance sync, so Home owes
                // that refresh too, once its own loads are out.
                Some(IdentityHubTab::Home) => match super::home::load_action(
                    &self.app_context,
                    &mut self.home_state,
                    &mut self.contacts_state,
                ) {
                    AppAction::None => {
                        super::tokens_tab::load_action(&self.app_context, &mut self.tokens_state)
                    }
                    home_load => home_load,
                },
                Some(IdentityHubTab::Contacts) => super::contacts::tab_load_action(
                    &self.app_context,
                    &mut self.contacts_state,
                    &mut self.profile_cache,
                ),
                Some(IdentityHubTab::Tokens) => {
                    super::tokens_tab::load_action(&self.app_context, &mut self.tokens_state)
                }
                _ => AppAction::None,
            };
        }
        if matches!(action, AppAction::None) {
            action = if let Some(task) = self.pending_avatars.pop() {
                AppAction::BackendTask(task)
            } else if self.app_context.connection_status().masternodes_ready() {
                // Profile reads verify proofs; sent before the masternode list
                // is synced they get the queried nodes banned.
                self.profile_cache.dispatch_pending()
            } else {
                AppAction::None
            };
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
        if let Some((network, identity_id, _)) = context.contacts_load()
            && !self.contacts_state.owns_load((network, identity_id))
        {
            return;
        }
        if self
            .tokens_state
            .record_result(context, &result, crate::utils::time::now_ms())
        {
            return;
        }
        self.home_state.record_payments(context, &result);
        if self
            .profile_cache
            .record_result(&self.app_context, context, &result)
            && let BackendTaskSuccessResult::DashPayProfile(snapshot) = &result
        {
            self.settings_tab.profile_refreshed(snapshot.owner);
        }
        if let BackendTaskSuccessResult::DashPayProfileUpdated(snapshot) = &result {
            handle_profile_updated(
                &self.app_context,
                &mut self.settings_tab,
                &mut self.profile_cache,
                context,
                snapshot,
            );
            return;
        }
        // A reload re-arms without cancelling the load before it, so lists
        // can answer out of order; an older one would undo a confirmed note.
        if let BackendTaskSuccessResult::DashPayContactsWithInfo { identity, contacts } = &result {
            if self
                .contacts_state
                .accepts_contacts_from(context.dispatch_id())
                && self.result_is_for_selected_identity(identity)
            {
                self.contacts_state
                    .record_loaded_contacts(contacts.clone(), context.dispatch_id());
            }
            return;
        }
        if let BackendTaskSuccessResult::DashPayContactInfoUpdated {
            identity,
            contact_id,
        } = &result
            && !self.is_own_contact_info_dispatch(context)
        {
            // Another screen's write: newer than any note the hub saved.
            self.contacts_state.forget_confirmed_note(
                self.app_context.network,
                identity,
                contact_id,
            );
            self.contacts_state.invalidate();
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
        // Another screen's token task changed the balances the hub shows.
        if matches!(result, BackendTaskSuccessResult::FetchedTokenBalances) {
            self.tokens_state
                .balances_refreshed_elsewhere(crate::utils::time::now_ms());
            return;
        }

        if self.handle_contact_request_result(&result) {
            return;
        }

        match &result {
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
            // A confirmed contactInfo write — an unhide or a private-note
            // save. Re-arm the load so the authoritative list replaces the
            // optimistic local edit.
            BackendTaskSuccessResult::DashPayContactInfoUpdated {
                identity,
                contact_id,
            } => {
                let key = ContactInfoTaskKey::Direct {
                    identity_id: *identity,
                    contact_id: *contact_id,
                };
                let network = self.contact_info_write_network(identity, contact_id);
                self.contacts_state
                    .finish_contact_info_write(network, identity, contact_id);
                if let Some(task) = self.forget_contact_info_task(&key) {
                    self.contacts_state.invalidate();
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        contact_info_saved_message(&task),
                        MessageType::Success,
                    );
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
                    self.profile_cache.reset();
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
        self.tokens_state.record_error(context, error);
        self.foreign_contact_info_error = context.dashpay_contact_info_target().is_some()
            && !self.is_own_contact_info_dispatch(context);
        // Wrapped errors retain the unreadable-details confirmation flow.
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
        // A superseded contact-list load says nothing about the newest one.
        if let Some((network, identity_id, kind)) = context.contacts_load()
            && self.contacts_state.owns_load((network, identity_id))
            && (kind != crate::model::dashpay::ContactsLoadKind::Contacts
                || self
                    .contacts_state
                    .accepts_contacts_from(context.dispatch_id()))
        {
            let syncing = matches!(error, TaskError::MasternodeListNotReady { .. });
            let delay = if syncing && !self.app_context.connection_status().masternodes_ready() {
                Some(std::time::Duration::ZERO)
            } else if syncing || contact_read_failure_is_transient(error) {
                Some(std::time::Duration::from_secs(5))
            } else {
                None
            };
            self.contacts_state
                .record_load_error(kind, delay.map(|delay| std::time::Instant::now() + delay));
            if let Some(delay) = delay {
                self.app_context.egui_ctx().request_repaint_after(delay);
            }
        }
        if matches!(error, TaskError::MasternodeListNotReady { .. }) {
            self.profile_cache.record_error_while_syncing(context);
        } else {
            self.profile_cache.record_error(context);
        }
        self.settings_tab.clear_pending_save(context);
        if let Some(offer) = self.settings_tab.usernames_card_mut().recovery_mut() {
            offer.absorb_error(context);
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        // The Tokens tab and Home's token slot show refresh failures inline.
        if self.tokens_state.take_swallowed_error() {
            return true;
        }
        // The status refresh runs in the background; a failure keeps the stored
        // status and retries on the next cadence tick instead of raising a banner.
        if matches!(error, TaskError::UsernameRequestRefreshFailed { .. }) {
            return true;
        }
        if self.handle_contact_request_error(error) {
            return matches!(
                contact_info_read_error_key(error),
                Some(key) if self.pending_contact_info_tasks.contains_key(&key)
            );
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

/// Whether a failed contact read may succeed on its own after the endpoint delay.
/// Extends the DAPI reachability check with a per-call deadline, which the
/// network client also treats as retryable. Kept local to contact reads: the
/// shared reachability check also decides all-addresses-exhausted reporting.
fn contact_read_failure_is_transient(error: &TaskError) -> bool {
    matches!(error, TaskError::DapiDeadlineExceeded { .. })
        || error.contains_dapi_reachability_failure()
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

/// Release the request and note-save guards of confirmations that will never
/// be answered: no task is left running for them.
fn release_guards_for_abandoned_confirmations<'a>(
    state: &mut super::contacts::ContactsState,
    network: Network,
    keys: impl IntoIterator<Item = &'a ContactInfoTaskKey>,
) {
    for key in keys {
        match key {
            ContactInfoTaskKey::Request(request_id) => state.release_request(request_id),
            ContactInfoTaskKey::Direct {
                identity_id,
                contact_id,
            } => state.release_contact_info_write(network, identity_id, contact_id),
        }
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
        DashPayTask::UpdateContactInfo { contact_id, .. } if is_note_save(task) => {
            let contact = shorten_id(&contact_id.to_string(Encoding::Base58));
            (
                "Save note and replace saved details?",
                format!(
                    "The saved details for contact {contact} cannot be read. Saving this note will clear the contact's saved nickname and accepted-account settings. You cannot undo this change."
                ),
            )
        }
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

pub(super) fn handle_profile_updated(
    app_context: &AppContext,
    settings: &mut SettingsTab,
    profiles: &mut super::profile_cache::ProfileCache,
    context: &BackendTaskContext,
    snapshot: &crate::model::dashpay::ProfileSnapshot,
) {
    if snapshot.network != app_context.network {
        return;
    }
    if !app_context.profile_snapshot_is_current(snapshot) {
        settings.retry_stale_profile_save(context);
        profiles.invalidate(snapshot.owner);
        return;
    }
    if let Some((display_name, bio, avatar_url)) = &snapshot.profile {
        profiles.record_saved(
            snapshot.owner,
            super::profile_cache::ProfileFields {
                display_name: display_name.clone(),
                bio: bio.clone(),
                avatar_url: avatar_url.clone(),
            },
        );
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

    #[test]
    fn stale_profile_success_does_not_show_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::context::test_support::test_app_context(dir.path());
        let ctx = app.egui_ctx();
        let mut settings = SettingsTab::new();
        let mut profiles = crate::ui::identity::profile_cache::ProfileCache::default();

        handle_profile_updated(
            &app,
            &mut settings,
            &mut profiles,
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

    /// Right after the app starts, the network's masternode list is not synced
    /// yet, so the first profile load fails. The hub must ask for the profile
    /// again once the list is ready, instead of leaving the identity without
    /// its profile until the user loads it again by hand.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_load_failed_while_syncing_is_retried_once_the_network_is_ready() {
        let (_dir, context) = wired_test_context().await;
        let owner = seed_user_identity(&context, 0x5A);
        context.set_selected_identity(Some(owner));
        context.connection_status().set_masternodes_ready(false);

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Vec<BackendTaskContext>)| {
                    if let AppAction::BackendTaskWithContext {
                        task: BackendTask::DashPayTask(task),
                        context,
                    } = state.0.ui(ui)
                        && matches!(*task, DashPayTask::LoadProfile { .. })
                    {
                        state.1.push(context);
                    }
                },
                (IdentityHubScreen::new(&context), Vec::new()),
            );
        harness.run_steps(4);
        assert!(harness.state().1.is_empty(), "no read before readiness");
        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(4);
        let first_load = {
            let loads = &harness.state().1;
            assert_eq!(loads.len(), 1, "the hub asks for the profile once");
            loads[0].clone()
        };
        // Readiness can drop again (a reconnect) while the read is running.
        context.connection_status().set_masternodes_ready(false);

        harness.state_mut().0.display_backend_task_error(
            &first_load,
            &TaskError::MasternodeListNotReady {
                source_error: Box::new(dash_sdk::Error::Generic(
                    "masternode list not yet synced".into(),
                )),
            },
        );
        harness.run_steps(4);
        assert_eq!(
            harness.state().1.len(),
            1,
            "no new load while the masternode list is still syncing"
        );

        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(4);
        assert_eq!(
            harness.state().1.len(),
            2,
            "the profile is asked for again once the masternode list is ready"
        );
        harness.run_steps(4);
        assert_eq!(harness.state().1.len(), 2, "and only once");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn home_contacts_retry_after_startup_sync_without_tab_navigation() {
        let (_dir, context) = wired_test_context().await;
        let owner = seed_user_identity(&context, 0x5B);
        context.set_selected_identity(Some(owner));
        context.connection_status().set_masternodes_ready(false);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, state: &mut (IdentityHubScreen, Vec<BackendTaskContext>)| {
                    if let AppAction::BackendTasksWithContexts(tasks, _) = state.0.ui(ui) {
                        for (task, _) in tasks {
                            if matches!(&task, BackendTask::DashPayTask(task)
                                if matches!(**task, DashPayTask::LoadContacts { .. }))
                            {
                                state.1.push(BackendTaskContext::from(&task));
                            }
                        }
                    }
                },
                (IdentityHubScreen::new(&context), Vec::new()),
            );
        harness.run_steps(4);
        assert!(harness.state().1.is_empty(), "no read before readiness");
        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(4);
        assert_eq!(harness.state().1.len(), 1);
        // Readiness can drop again (a reconnect) while the read is running.
        context.connection_status().set_masternodes_ready(false);
        let first = harness.state().1[0].clone();
        harness.state_mut().0.display_backend_task_error(
            &first,
            &TaskError::MasternodeListNotReady {
                source_error: Box::new(dash_sdk::Error::Generic("syncing".into())),
            },
        );
        harness.run_steps(4);
        assert_eq!(harness.state().1.len(), 1, "wait for network readiness");
        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(4);
        assert_eq!(
            harness.state().1.len(),
            2,
            "Home must fetch existing contacts after startup recovers"
        );
        harness.run_steps(4);
        assert_eq!(harness.state().1.len(), 2, "retry only once");
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
                        AppAction::BackendTasksWithContexts(tasks, _) => {
                            tasks.into_iter().map(|(task, _)| task).collect()
                        }
                        _ => Vec::new(),
                    };
                    for task in tasks {
                        if let BackendTask::DashPayTask(task) = task {
                            state.1.push(match *task {
                                DashPayTask::LoadContacts { .. } => "contacts",
                                DashPayTask::LoadContactRequests { .. } => "requests",
                                DashPayTask::LoadProfile { .. } => "profile",
                                DashPayTask::LoadPaymentHistory { .. } => "payments",
                                _ => continue,
                            });
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
            harness.state().0.contacts_state.load_is_due(),
            "Home asked for its contacts and was held, not skipped"
        );
        {
            use egui_kittest::kittest::Queryable;
            assert!(harness.query_by_label("Loading contacts…").is_some());
        }

        context.connection_status().set_masternodes_ready(true);
        harness.run_steps(6);
        let mut reads = harness.state().1.clone();
        reads.sort_unstable();
        assert_eq!(reads, ["contacts", "payments", "profile", "requests"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn home_contacts_retry_only_matching_transient_reads_and_keep_paid_guards() {
        let (_dir, context) = wired_test_context().await;
        context.connection_status().set_masternodes_ready(true);
        let owner = seed_user_identity(&context, 0x5C);
        let identity = context.get_identity_by_id(&owner).unwrap().unwrap();
        let request_id = id(0x5D);
        for requests in [false, true] {
            let task = BackendTask::DashPayTask(Box::new(if requests {
                DashPayTask::LoadContactRequests {
                    identity: identity.clone(),
                }
            } else {
                DashPayTask::LoadContacts {
                    identity: identity.clone(),
                }
            }));
            let own = BackendTaskContext::for_dispatch(&task);
            for (dispatch, transient, retries) in [
                (own.clone(), true, true),
                (own.clone(), false, false),
                (BackendTaskContext::Other, true, false),
                (
                    BackendTaskContext::DashPayContactsLoad {
                        network: Network::Mainnet,
                        identity_id: owner,
                        kind: crate::model::dashpay::ContactsLoadKind::Contacts,
                    },
                    true,
                    false,
                ),
                (
                    BackendTaskContext::DashPayContactsLoad {
                        network: Network::Testnet,
                        identity_id: id(0x6F),
                        kind: crate::model::dashpay::ContactsLoadKind::Contacts,
                    },
                    true,
                    false,
                ),
            ] {
                let mut hub = IdentityHubScreen::new(&context);
                hub.contacts_state.reconcile_owner(&identity);
                assert!(hub.contacts_state.claim_load());
                assert!(hub.contacts_state.begin_request(request_id));
                let before = std::time::Instant::now();
                let error = if transient && requests {
                    TaskError::MasternodeListNotReady {
                        source_error: Box::new(dash_sdk::Error::Generic("syncing".into())),
                    }
                } else if transient {
                    TaskError::DapiUnavailable {
                        source_error: Box::new(dash_sdk::Error::Generic("unavailable".into())),
                    }
                } else {
                    TaskError::NoIdentitiesFound
                };
                hub.display_backend_task_error(&dispatch, &error);
                hub.contacts_state.retry_load_if_due(before);
                assert!(
                    !hub.contacts_state.load_is_due(),
                    "do not spin on endpoint failures"
                );
                hub.contacts_state
                    .retry_load_if_due(before + std::time::Duration::from_secs(6));
                assert_eq!(hub.contacts_state.load_is_due(), retries);
                assert!(hub.contacts_state.is_in_flight(&request_id));
                assert!(!hub.contacts_state.begin_request(request_id));
            }
        }
    }

    /// A contact read that ran past its per-call deadline is a temporary
    /// failure the network client itself retries, so Home retries it too
    /// after the endpoint delay instead of leaving the load stranded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn home_contacts_retry_a_read_that_hit_the_call_deadline() {
        use dash_sdk::dapi_client::DapiClientError;
        use dash_sdk::dapi_client::transport::TransportError;
        let (_dir, context) = wired_test_context().await;
        context.connection_status().set_masternodes_ready(true);
        let owner = seed_user_identity(&context, 0x5E);
        let identity = context.get_identity_by_id(&owner).unwrap().unwrap();
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadContacts {
            identity: identity.clone(),
        }));
        let mut hub = IdentityHubScreen::new(&context);
        hub.contacts_state.reconcile_owner(&identity);
        assert!(hub.contacts_state.claim_reads().is_some());
        let error = TaskError::from(dash_sdk::Error::DapiClientError(
            DapiClientError::Transport(TransportError::Grpc(
                dash_sdk::dapi_grpc::tonic::Status::deadline_exceeded("deadline"),
            )),
        ));
        assert!(matches!(error, TaskError::DapiDeadlineExceeded { .. }));
        let before = std::time::Instant::now();
        hub.display_backend_task_error(&BackendTaskContext::for_dispatch(&task), &error);
        hub.contacts_state.retry_load_if_due(before);
        assert!(!hub.contacts_state.load_is_due(), "wait the endpoint delay");
        hub.contacts_state
            .retry_load_if_due(before + std::time::Duration::from_secs(6));
        assert_eq!(
            hub.contacts_state.claim_reads(),
            Some(std::collections::BTreeSet::from([
                crate::model::dashpay::ContactsLoadKind::Contacts
            ])),
            "a deadline failure must be retried"
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authoritative_profile_refresh_rebases_seeded_settings_before_save() {
        use crate::model::dashpay::ProfileSnapshot;
        use egui_kittest::{Harness, kittest::Queryable};
        let (_dir, context) = wired_test_context().await;
        let identity = seed_cached_profile(&context).await;
        let owner = identity.identity.id();
        for absent in [false, true] {
            for dirty in [false, true] {
                let mut hub = IdentityHubScreen::new(&context);
                hub.profile_cache.seed_cached(&context, &identity);
                let AppAction::BackendTaskWithContext { context: load, .. } =
                    hub.profile_cache.dispatch_pending()
                else {
                    panic!("load dispatch")
                };
                let mut editor = Harness::builder()
                    .with_size(egui::vec2(1100.0, 1100.0))
                    .build_ui_state(
                        |ui, state: &mut (IdentityHubScreen, Option<BackendTask>)| {
                            let hub = &mut state.0;
                            if let AppAction::BackendTaskWithContext { task, .. } =
                                hub.settings_tab.render(
                                    ui,
                                    &hub.app_context,
                                    hub.app_context.resolve_selected_identity().as_ref(),
                                    &mut hub.profile_cache,
                                )
                            {
                                state.1 = Some(task);
                            }
                        },
                        (hub, None),
                    );
                editor.run();
                if dirty {
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
                }
                let profile = (!absent).then(|| {
                    (
                        "Authoritative name".into(),
                        "New biography".into(),
                        "https://example.com/new.png".into(),
                    )
                });
                let result = BackendTaskSuccessResult::DashPayProfile(ProfileSnapshot {
                    network: context.network,
                    owner,
                    revision: context.identity_profile_revision(owner),
                    profile,
                });
                editor
                    .state_mut()
                    .0
                    .display_backend_task_result(&load, result);
                editor.run();
                if !dirty {
                    editor.get_by_label("Save social profile").click();
                    editor.run();
                    assert!(
                        editor.state().1.is_none(),
                        "refreshed fields must be a clean baseline"
                    );
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
                } else if absent {
                    "draft"
                } else {
                    "Authoritative name draft"
                };
                assert_eq!(display_name.as_deref(), Some(expected_name));
                assert_eq!(bio.as_deref(), (!absent).then_some("New biography"));
                assert_eq!(
                    avatar_url.as_deref(),
                    (!absent).then_some("https://example.com/new.png")
                );
            }
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
    async fn cached_profile_primes_avatar_without_overwriting_saved_or_absent_profile() {
        let (_dir, context) = wired_test_context().await;
        let identity = seed_cached_profile(&context).await;
        let id = identity.identity.id();
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
            .state_mut()
            .profile_cache
            .get_or_request(&identity)
            .and_then(Option::as_ref)
            .expect("persisted profile is immediately available offline");
        assert_eq!(fields.avatar_url, "https://example.com/cached.png");
        harness.state_mut().profile_cache.record_saved(
            id,
            super::super::profile_cache::ProfileFields {
                display_name: "Saved".into(),
                ..Default::default()
            },
        );
        harness.run_steps(2);
        assert_eq!(
            harness
                .state_mut()
                .profile_cache
                .get_or_request(&identity)
                .unwrap()
                .as_ref()
                .unwrap()
                .display_name,
            "Saved"
        );
        let cache = &mut harness.state_mut().profile_cache;
        cache.reset();
        cache.get_or_request(&identity);
        let AppAction::BackendTaskWithContext { context: load, .. } = cache.dispatch_pending()
        else {
            panic!("profile load")
        };
        assert!(cache.record_result(
            &context,
            &load,
            &BackendTaskSuccessResult::DashPayProfile(crate::model::dashpay::ProfileSnapshot {
                network: context.network,
                owner: id,
                revision: context.identity_profile_revision(id),
                profile: None
            })
        ));
        harness.run_steps(2);
        assert!(
            harness
                .state_mut()
                .profile_cache
                .get_or_request(&identity)
                .unwrap()
                .is_none(),
            "authoritative absence must not be replaced by old persisted fields"
        );
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
            network: screen.app_context.network,
            identity_ids: vec![fallback],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
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
            network: screen.app_context.network,
            identity_ids: vec![unloaded],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
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
            network: screen.app_context.network,
            identity_ids: vec![first],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
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
            network: screen.app_context.network,
            identity_ids: vec![elsewhere],
            associated_cleanup_failed: false,
            local_data_cleanup_failed: false,
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

        release_guards_for_abandoned_confirmations(&mut state, Network::Testnet, &keys);

        assert!(state.is_in_flight(&id(1)));
        assert!(
            !state.is_in_flight(&id(2)),
            "a discarded confirmation has no running task left to protect"
        );
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
    fn a_saved_note_and_an_unhide_confirm_differently() {
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

        let (title, message) = contact_info_confirmation_copy(&note);
        assert_eq!(title, "Save note and replace saved details?");
        assert!(
            message.contains("nickname") && !message.contains("Unhiding"),
            "the overwrite warning names what a note save clears: {message}"
        );
        let (title, _) = contact_info_confirmation_copy(&unhide);
        assert_eq!(title, "Unhide contact and replace saved details?");
    }

    #[test]
    fn abandoning_a_note_save_confirmation_releases_its_guard() {
        let mut state = ContactsState::default();
        assert!(state.begin_request(id(2)));
        state.record_contacts(vec![crate::backend_task::dashpay::ContactData {
            identity_id: id(4),
            nickname: None,
            note: None,
            is_hidden: false,
            details_unreadable: false,
            account_reference: 0,
            username: None,
            display_name: None,
            avatar_url: None,
            bio: None,
        }]);
        state.select_contact(&id(4));
        *state.detail_mut().note_mut(&id(4)).expect("draft") = "Bao".into();
        assert!(state.detail_mut().begin_save(id(3), id(4)).is_some());

        release_guards_for_abandoned_confirmations(
            &mut state,
            Network::Testnet,
            &[ContactInfoTaskKey::Direct {
                identity_id: id(3),
                contact_id: id(4),
            }],
        );

        assert!(!state.detail().is_writing(&id(3), &id(4)));
        assert!(state.is_in_flight(&id(2)), "unrelated request guards stay");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_note_save_releases_only_its_own_guard() {
        let (_temp_dir, app_context) = wired_test_context().await;
        let mut screen = IdentityHubScreen::new(&app_context);
        let state = &mut screen.contacts_state;
        state.record_contacts(vec![crate::backend_task::dashpay::ContactData {
            identity_id: id(4),
            nickname: None,
            note: None,
            is_hidden: false,
            details_unreadable: false,
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
            screen.contacts_state.record_contacts(vec![
                crate::backend_task::dashpay::ContactData {
                    identity_id: contact,
                    nickname: None,
                    note: None,
                    is_hidden: false,
                    details_unreadable: false,
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

    /// r4316990948: load A reads the old note and is still running when the
    /// save lands and load B starts. B answers first with the saved note; A
    /// answering afterwards must not put the old note back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stale_contacts_load_does_not_undo_a_confirmed_note() {
        use crate::backend_task::dashpay::ContactData;
        use crate::model::dashpay::ContactInfoUpdate;

        let (_temp_dir, app_context) = wired_test_context().await;
        let identity = crate::context::test_support::user_identity_with_signing_key(
            id(0x31),
            Network::Testnet,
            false,
        );
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed identity");
        app_context.set_selected_identity(Some(identity.identity.id()));
        let (owner, contact) = (identity.identity.id(), id(4));
        let listed = |note: Option<&str>| {
            vec![ContactData {
                identity_id: contact,
                nickname: None,
                note: note.map(Into::into),
                is_hidden: false,
                details_unreadable: false,
                account_reference: 0,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            }]
        };
        let contacts_load = |screen: &mut IdentityHubScreen| {
            let AppAction::BackendTasksWithContexts(tasks, _) =
                super::super::contacts::load_action(&identity, &mut screen.contacts_state)
            else {
                panic!("the contact loads are dispatched");
            };
            tasks
                .into_iter()
                .find_map(|(task, context)| {
                    matches!(&task, BackendTask::DashPayTask(t)
                        if matches!(**t, DashPayTask::LoadContacts { .. }))
                    .then_some(context)
                })
                .expect("a contact list load")
        };

        let mut screen = IdentityHubScreen::new(&app_context);
        screen.contacts_state.reconcile_owner(&identity);
        screen.contacts_state.record_contacts(listed(None));
        screen.contacts_state.select_contact(&contact);
        let load_a = contacts_load(&mut screen);

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
                identity: identity.clone(),
                contact_id: contact,
                update: ContactInfoUpdate::note(Some("Bao".into())),
            },
        )));
        screen.capture_contact_info_update(&action);
        let AppAction::BackendTaskWithContext { context: write, .. } =
            screen.bind_contact_info_dispatch(action)
        else {
            panic!("the write is sent under its own dispatch");
        };
        screen.display_backend_task_result(
            &write,
            BackendTaskSuccessResult::DashPayContactInfoUpdated {
                identity: owner,
                contact_id: contact,
            },
        );
        let load_b = contacts_load(&mut screen);

        let contacts_result = |note| BackendTaskSuccessResult::DashPayContactsWithInfo {
            identity: owner,
            contacts: listed(note),
        };
        screen.display_backend_task_result(&load_b, contacts_result(Some("Bao")));
        screen.display_backend_task_result(&load_a, contacts_result(None));

        assert_eq!(
            screen
                .contacts_state
                .selected_contact()
                .and_then(|c| c.note.as_deref()),
            Some("Bao"),
            "the older load must not put the old note back"
        );
        assert_eq!(
            screen
                .contacts_state
                .detail_mut()
                .note_mut(&contact)
                .map(|draft| draft.clone()),
            Some("Bao".into())
        );
    }

    /// One event in the life of a confirmed note; see `ConfirmedNote`.
    #[derive(Debug, Clone, Copy)]
    enum NoteEvent {
        /// The hub's write succeeds.
        Saved,
        /// The hub's write fails.
        Failed,
        /// The hub's write succeeds after the hub moved to another network.
        SavedOnTheNetworkLeft,
        /// The legacy Contact Details screen saves the same contact.
        SavedElsewhere,
        /// A list from the load dispatched before the save.
        ListFromBefore(&'static str),
        /// A list from a load dispatched after the save.
        ListFromAfter(&'static str),
        IdentitySwitch,
        /// The lists now belong to the same owner on another network.
        NetworkSwitch,
    }

    /// Every event that creates, keeps or retires a confirmed note, with the
    /// note the hub shows after it. The contact's note was "Old"; the hub
    /// saves "Mine".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_confirmed_note_lifecycle() {
        use NoteEvent::*;
        let cases: &[(&str, &[NoteEvent], &str)] = &[
            (
                "a list from before the save",
                &[Saved, ListFromBefore("Old")],
                "Mine",
            ),
            (
                "a read lagging behind the save",
                &[Saved, ListFromAfter("Old")],
                "Mine",
            ),
            (
                "the saved note read back retires it",
                &[Saved, ListFromAfter("Mine"), ListFromAfter("Old")],
                "Old",
            ),
            (
                "a newer note saved elsewhere",
                &[Saved, ListFromAfter("Theirs")],
                "Theirs",
            ),
            (
                "another screen's save retires it",
                &[Saved, SavedElsewhere, ListFromAfter("Old")],
                "Old",
            ),
            (
                "a failed save confirms nothing",
                &[Failed, ListFromAfter("Old")],
                "Old",
            ),
            (
                "an identity switch keeps it",
                &[Saved, IdentitySwitch, ListFromBefore("Old")],
                "Mine",
            ),
            (
                "a save landing after an identity switch waits out a lagging read",
                &[IdentitySwitch, Saved, ListFromAfter("Old")],
                "Mine",
            ),
            (
                "another network's lists are not overridden",
                &[Saved, NetworkSwitch, ListFromAfter("Old")],
                "Old",
            ),
            (
                "a save from the network left changes nothing here",
                &[SavedOnTheNetworkLeft],
                "Old",
            ),
        ];
        let mut wrong = Vec::new();
        for (name, events, shown) in cases {
            let got = confirmed_note_after(events).await;
            if got.as_deref() != Some(*shown) {
                wrong.push(format!("{name}: shows {got:?}, expected {shown:?}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// The note the hub shows for the contact after `events`, asserting that
    /// the hub's write was settled.
    async fn confirmed_note_after(events: &[NoteEvent]) -> Option<String> {
        use crate::backend_task::dashpay::ContactData;
        use crate::model::dashpay::ContactInfoUpdate;

        let (_temp_dir, app_context) = wired_test_context().await;
        let identity = crate::context::test_support::user_identity_with_signing_key(
            id(0x31),
            Network::Testnet,
            false,
        );
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed identity");
        app_context.set_selected_identity(Some(identity.identity.id()));
        let (owner, contact) = (identity.identity.id(), id(4));
        let list = |note: &str| BackendTaskSuccessResult::DashPayContactsWithInfo {
            identity: owner,
            contacts: vec![ContactData {
                identity_id: contact,
                nickname: None,
                note: Some(note.into()),
                is_hidden: false,
                details_unreadable: false,
                account_reference: 0,
                username: None,
                display_name: None,
                avatar_url: None,
                bio: None,
            }],
        };
        let contacts_load = |screen: &mut IdentityHubScreen, on_screen: &QualifiedIdentity| {
            screen.contacts_state.invalidate();
            let AppAction::BackendTasksWithContexts(tasks, _) =
                super::super::contacts::load_action(on_screen, &mut screen.contacts_state)
            else {
                panic!("the contact loads are dispatched");
            };
            tasks
                .into_iter()
                .find_map(|(task, context)| {
                    matches!(&task, BackendTask::DashPayTask(t)
                        if matches!(**t, DashPayTask::LoadContacts { .. }))
                    .then_some(context)
                })
                .expect("a contact list load")
        };

        let mut screen = IdentityHubScreen::new(&app_context);
        screen.contacts_state.reconcile_owner(&identity);
        // The identity whose lists are on screen; a network switch changes it.
        let mut on_screen = identity.clone();
        let before = contacts_load(&mut screen, &on_screen);
        screen.display_backend_task_result(&before, list("Old"));
        screen.contacts_state.select_contact(&contact);
        *screen
            .contacts_state
            .detail_mut()
            .note_mut(&contact)
            .expect("draft") = "Mine".into();
        assert!(
            screen
                .contacts_state
                .detail_mut()
                .begin_save(owner, contact)
                .is_some()
        );
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::UpdateContactInfo {
            identity: identity.clone(),
            contact_id: contact,
            update: ContactInfoUpdate::note(Some("Mine".into())),
        }));
        let action = AppAction::BackendTask(task.clone());
        screen.capture_contact_info_update(&action);
        let AppAction::BackendTaskWithContext { context: write, .. } =
            screen.bind_contact_info_dispatch(action)
        else {
            panic!("the write is sent under its own dispatch");
        };
        let saved = BackendTaskSuccessResult::DashPayContactInfoUpdated {
            identity: owner,
            contact_id: contact,
        };

        for event in events {
            match *event {
                NoteEvent::Saved => screen.display_backend_task_result(&write, saved.clone()),
                NoteEvent::Failed => {
                    let error = TaskError::DashPayContactInfoActionFailed {
                        identity_id: owner,
                        contact_id: contact,
                        source: Box::new(TaskError::WalletLocked),
                    };
                    screen.display_backend_task_error(&write, &error);
                    screen.display_task_error(&error);
                }
                NoteEvent::SavedOnTheNetworkLeft => {
                    for record in screen.contact_info_dispatches.values_mut() {
                        record.1 = Network::Mainnet;
                    }
                    screen.display_backend_task_result(&write, saved.clone());
                }
                NoteEvent::SavedElsewhere => screen.display_backend_task_result(
                    &BackendTaskContext::for_dispatch(&task),
                    saved.clone(),
                ),
                NoteEvent::ListFromBefore(note) => {
                    screen.display_backend_task_result(&before, list(note))
                }
                NoteEvent::ListFromAfter(note) => {
                    let after = contacts_load(&mut screen, &on_screen);
                    screen.display_backend_task_result(&after, list(note));
                }
                NoteEvent::IdentitySwitch => screen.reset_contacts_for_identity_change(),
                NoteEvent::NetworkSwitch => {
                    on_screen.network = Network::Mainnet;
                    screen.contacts_state.reconcile_owner(&on_screen);
                }
            }
        }
        assert!(
            !screen.contacts_state.detail().is_writing(&owner, &contact),
            "the write is settled"
        );
        screen
            .contacts_state
            .matching_contacts()
            .find(|c| c.identity_id == contact)
            .and_then(|c| c.note.clone())
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
