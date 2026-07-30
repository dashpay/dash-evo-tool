//! The Masternodes list root screen (FR-2 empty state, FR-3 card grid, FR-7
//! refresh). Reuses the identity onboarding empty-state pattern and the
//! identity-picker card visual language via [`MasternodeCard`]; a card click
//! opens the detail view.

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, RichText};

use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::identity_load_registry::{IdentityLoadPhase, IdentityLoadToken};
use crate::model::contested_name::MasternodeContestSummary;
use crate::model::masternode_input::decode_identity_id;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, MasternodeKeyPresence};
use crate::model::user_role::UserRole;
use crate::model::wallet_association::WalletAssociation;
use crate::ui::components::MessageBanner;
use crate::ui::components::global_nav_switcher::GlobalNavEffect;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::legacy_recovery_section::completion_message;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel_with_global_nav_capturing;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::identity::picker::compute_column_count;
use crate::ui::masternodes::card::{MasternodeCard, card_heading};
use crate::ui::masternodes::detail_screen::{DetailOutcome, MasternodeDetailView};
use crate::ui::masternodes::load_form::{LoadFormOutcome, MasternodeLoadForm};
use crate::ui::state::global_nav::PageNavSpec;
use crate::ui::state::masternodes_view::{masternodes_page_nav_spec, node_pill_item};
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::{MessageType, RootScreenType, ScreenLike};

/// Minimum horizontal gap between cards in the grid (matches the identity
/// picker grid).
const GRID_GAP: f32 = 16.0;

/// Pre-resolved display data for one masternode/evonode card. Computed at
/// reload time so the per-frame render never touches the database.
struct NodeCardData {
    node_id: Identifier,
    node_id_short: String,
    alias: Option<String>,
    balance_credits: u64,
    node_type: IdentityType,
    key_presence: MasternodeKeyPresence,
    contest_summary: MasternodeContestSummary,
    status: IdentityStatus,
}

/// Which sub-view of the Masternodes section is showing. The detail view (B5a)
/// and the page-scoped view-state machine (B7) extend this enum.
enum MasternodesView {
    /// Empty state or card grid.
    List,
    /// The masternode/evonode load form (FR-4).
    Load(Box<MasternodeLoadForm>),
    /// A node's detail / voting view (FR-5).
    Detail(Box<MasternodeDetailView>),
}

/// A load this screen dispatched and has not yet seen finish. The token names
/// that one load, so a later load of the same node — dispatched from anywhere —
/// can never be mistaken for it.
#[derive(Debug, Clone, Copy)]
struct PendingLoad {
    identity_id: Identifier,
    token: IdentityLoadToken,
}

/// Root screen for the Masternodes section.
pub struct MasternodesScreen {
    pub app_context: Arc<AppContext>,
    /// Cached card data for the active network, refreshed on arrival, on
    /// `refresh`, and on the Refresh button.
    nodes: Vec<NodeCardData>,
    /// The active sub-view (list / load / detail).
    view: MasternodesView,
    /// The load this screen dispatched and has not yet seen finish, identified by
    /// the ProTxHash that was *submitted* — never re-read from the form, whose
    /// fields stay editable while the load runs. Gates the load entry points
    /// (`+ Load`, the empty-state CTA, the form's submit button) so a load cannot
    /// be dispatched twice.
    ///
    /// Cleared by [`reconcile_pending_load`](Self::reconcile_pending_load) once
    /// the load's own task reports it finished — never on a guess from the store
    /// or from which callbacks happened to fire. `None` when there is nothing to
    /// gate: a submitted ProTxHash that did not parse (the backend rejects such a
    /// load before touching the store), or a node another caller is already
    /// loading (that load owns the record, and this screen's dispatch is rejected
    /// with a message rather than gating on someone else's work).
    ///
    /// This gate is the UX layer — it keeps the buttons honest. The exclusion
    /// that actually protects the store is the load task's own claim on the
    /// identity, so a duplicate dispatch that slips through fails closed with
    /// [`TaskError::IdentityLoadInProgress`](crate::backend_task::error::TaskError::IdentityLoadInProgress)
    /// instead of racing.
    pending_load: Option<PendingLoad>,
}

#[cfg(test)]
impl MasternodesScreen {
    /// Return card headings in the same order shown by the grid and node pill.
    pub(crate) fn node_headings_for_test(&self) -> Vec<String> {
        self.nodes
            .iter()
            .map(|node| card_heading(node.alias.as_deref(), &node.node_id_short))
            .collect()
    }
}

impl MasternodesScreen {
    /// Construct the Masternodes screen. Follows the project convention:
    /// constructors handle errors internally and return `Self` (degraded to an
    /// empty list if the read fails; the empty state renders).
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let mut screen = Self {
            app_context: app_context.clone(),
            nodes: Vec::new(),
            view: MasternodesView::List,
            pending_load: None,
        };
        screen.reload();
        screen
    }

    /// Re-read the loaded masternode/evonode identities and their DPNS contest
    /// summaries from the local store. A read failure degrades to an empty list
    /// rather than surfacing a technical error — the empty state is a safe,
    /// meaningful fallback.
    fn reload(&mut self) {
        let identities = self
            .app_context
            .load_local_masternode_identities()
            .unwrap_or_default();

        self.nodes = identities
            .into_iter()
            .map(|qi| {
                let node_id = qi.identity.id();
                let node_id_short = shorten_id(&node_id.to_string(Encoding::Hex));
                let voter_id = qi
                    .associated_voter_identity
                    .as_ref()
                    .map(|(identity, _)| identity.id());
                let contest_summary = self
                    .app_context
                    .masternode_contest_summary(voter_id)
                    .unwrap_or_default();
                NodeCardData {
                    node_id,
                    node_id_short,
                    alias: qi.alias.clone(),
                    balance_credits: qi.identity.balance(),
                    node_type: qi.identity_type,
                    key_presence: qi.masternode_key_presence(),
                    contest_summary,
                    status: qi.status,
                }
            })
            .collect();
        self.nodes.sort_by_key(|node| {
            card_heading(node.alias.as_deref(), &node.node_id_short).to_lowercase()
        });
    }

    /// Settle the submitted load against the phase its own task reported, and
    /// release the gate once that load is finished. The single place this screen
    /// decides a load is over — its result and error callbacks fire only while it
    /// is the visible screen, so a load that ran while the user was elsewhere is
    /// settled here, on the next arrival.
    ///
    /// Only the phase decides. Neither of the tempting proxies is sound: a load is
    /// outstanding from the moment it is dispatched, before its task claims the
    /// identity, and a *persisted node* does not mean success — the load inserts
    /// the node before sealing its keys, so a failed seal leaves the node stored
    /// by a load that errored. A failed load therefore keeps its form, and every
    /// field in it, ready for a corrected resubmit; the error was already shown as
    /// a global banner wherever it landed.
    fn reconcile_pending_load(&mut self) {
        let Some(pending) = self.pending_load else {
            return;
        };
        let phase = self
            .app_context
            .identity_load_phase(&pending.identity_id, pending.token);
        if phase.is_some_and(IdentityLoadPhase::is_outstanding) {
            return;
        }
        self.pending_load = None;
        if phase == Some(IdentityLoadPhase::Loaded) && matches!(self.view, MasternodesView::Load(_))
        {
            self.view = MasternodesView::List;
        }
    }

    /// Reset the screen after a network switch. A load form or detail
    /// view left open belongs to the previous network's node — keeping it
    /// actionable would let the user submit a cross-network operation. Drop back
    /// to the List view and reload from the now-active network's local store.
    pub fn reset_for_network_change(&mut self) {
        self.view = MasternodesView::List;
        self.pending_load = None;
        self.reload();
    }

    /// Build a fresh load form, attaching the Testnet Fill-Random fixture only
    /// on Testnet (loaded once here, not per frame).
    fn new_load_form(&self) -> Box<MasternodeLoadForm> {
        let fixture = if self.app_context.network == dash_sdk::dpp::dashcore::Network::Testnet {
            crate::ui::masternodes::testnet_fixture::load_testnet_nodes()
        } else {
            None
        };
        Box::new(MasternodeLoadForm::new().with_testnet_fixture(fixture))
    }

    /// Render the centered empty state (FR-2). Returns the action produced by
    /// the primary CTA.
    fn render_empty_state(&mut self, ui: &mut egui::Ui) -> AppAction {
        let dark_mode = ui.style().visuals.dark_mode;

        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.label(
                RichText::new("No masternodes loaded")
                    .size(22.0)
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(12.0);
            ui.label(
                RichText::new(
                    "Load a masternode or evonode to vote on DPNS name contests and manage its \
                     owner and payout keys.",
                )
                .size(14.0)
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(20.0);
            // Gate the CTA while a load is in flight.
            if self.pending_load.is_some() {
                ui.spinner();
                ui.add_enabled(false, egui::Button::new("Loading…"));
            } else if ComponentStyles::add_primary_button(ui, "Load a masternode").clicked() {
                self.view = MasternodesView::Load(self.new_load_form());
            }
            ui.add_space(12.0);
            ui.label(
                RichText::new(
                    "Have your node's ProTxHash to hand. Keys are optional — a node loads \
                     read-only without them.",
                )
                .size(12.0)
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(48.0);
        });

        AppAction::None
    }

    /// Render the responsive card grid (FR-3). Sets `selected_node` when a card
    /// is clicked (routed to the detail view in B5a/B7).
    fn render_card_grid(&mut self, ui: &mut egui::Ui) -> AppAction {
        let available_width = ui.available_width();
        let columns = compute_column_count(available_width).max(1);
        let count = self.nodes.len();
        // Capture the clicked node id locally so the render loop only borrows
        // `self.nodes` immutably; `selected_node` is written after the loop.
        let mut clicked: Option<Identifier> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            for row_start in (0..count).step_by(columns) {
                ui.horizontal(|ui| {
                    for idx in row_start..(row_start + columns).min(count) {
                        if idx > row_start {
                            ui.add_space(GRID_GAP);
                        }
                        let node = &self.nodes[idx];
                        let card = MasternodeCard::new(
                            node.node_id.to_string(Encoding::Hex),
                            node.node_id_short.clone(),
                            node.node_type,
                            node.key_presence,
                            node.contest_summary,
                            node.status,
                        )
                        .with_alias(node.alias.clone())
                        .with_balance_credits(node.balance_credits);
                        if card.show(ui).clicked {
                            clicked = Some(node.node_id);
                        }
                    }
                });
                ui.add_space(GRID_GAP);
            }
        });

        if let Some(node_id) = clicked {
            self.open_detail(node_id);
        }

        AppAction::None
    }

    /// The node the page currently operates on — the one whose detail view is
    /// open. The list and load views operate on no single node.
    fn selected_node_id(&self) -> Option<Identifier> {
        match &self.view {
            MasternodesView::Detail(detail) => Some(detail.node_id()),
            MasternodesView::List | MasternodesView::Load(_) => None,
        }
    }

    /// The page's global-nav spec: a read-only wallet indicator plus a node pill
    /// listing every loaded node and showing the one in view. Derived from the
    /// page's own state each frame, which is what keeps the pill and the card
    /// grid two-way bound (FR-GLOBAL-NAV-3).
    fn nav_spec(&self) -> PageNavSpec {
        let items = self
            .nodes
            .iter()
            .map(|node| {
                node_pill_item(
                    node.node_id,
                    node.alias.as_deref(),
                    &node.node_id_short,
                    node.node_type,
                )
            })
            .collect();
        masternodes_page_nav_spec(items, self.selected_node_id(), self.wallet_association())
    }

    /// The wallet association shown for the node in view. Masternodes and
    /// evonodes are loaded from raw owner/voting/payout keys, never derived from
    /// an HD wallet on this device, so none has an owning wallet hash. Resolved
    /// through [`WalletAssociation::resolve`] so the indicator stays correct if a
    /// wallet-derived node type is ever added.
    fn wallet_association(&self) -> WalletAssociation {
        WalletAssociation::resolve(None, &[])
    }

    /// Consume the global-nav effect this page is bound to: a node picked from
    /// the node pill opens its detail view. Every other effect (segment-1
    /// navigation, a wallet switch) is already applied by the shared applier.
    fn apply_nav_effect(&mut self, effect: GlobalNavEffect) {
        if let GlobalNavEffect::SelectPageObject(node_id) = effect {
            self.open_detail(node_id);
        }
    }

    /// Whether `node_id` names something this page is currently showing: the
    /// open detail view's node, or one of the cards in the list.
    ///
    /// The attribution rule for a backend result that reaches this screen only
    /// because it is the visible one. An identity nothing on screen names —
    /// another node, or a `User` identity from the Identities section — has no
    /// row here to update and no story to tell here.
    fn shows_node(&self, node_id: Identifier) -> bool {
        match &self.view {
            MasternodesView::Detail(detail) => detail.node_id() == node_id,
            _ => self.nodes.iter().any(|node| node.node_id == node_id),
        }
    }

    /// Open the detail view for `node_id`. Loads the node's full
    /// `QualifiedIdentity` from the local store; a lookup miss leaves the list
    /// view unchanged.
    fn open_detail(&mut self, node_id: Identifier) {
        let Ok(identities) = self.app_context.load_local_masternode_identities() else {
            return;
        };
        if let Some(identity) = identities
            .into_iter()
            .find(|qi| qi.identity.id() == node_id)
        {
            self.view = MasternodesView::Detail(Box::new(MasternodeDetailView::new(
                &self.app_context,
                identity,
            )));
        }
    }

    /// Render the detail view; map its outcome to navigation / a reused screen.
    fn render_detail_view(
        &mut self,
        ui: &mut egui::Ui,
        network_accent: egui::Color32,
    ) -> AppAction {
        let outcome = match &mut self.view {
            MasternodesView::Detail(detail) => detail.show(ui, network_accent),
            _ => return AppAction::None,
        };
        match outcome {
            DetailOutcome::None => AppAction::None,
            DetailOutcome::Back => {
                self.view = MasternodesView::List;
                AppAction::None
            }
            DetailOutcome::Removed => {
                self.view = MasternodesView::List;
                self.reload();
                AppAction::None
            }
            DetailOutcome::Forward(action) => *action,
        }
    }

    /// Render the list view: toolbar (`+ Load`, `Refresh`) + empty state or grid.
    fn render_list_view(&mut self, ui: &mut egui::Ui, network_accent: egui::Color32) -> AppAction {
        let mut inner = AppAction::None;

        // Top-right toolbar: `+ Load` (FR-4 entry) + `Refresh` (FR-7).
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ComponentStyles::add_toolbar_button(ui, "Refresh", network_accent).clicked() {
                    // Re-read the local cache immediately (optimistic) AND
                    // dispatch a network re-fetch of every loaded node plus the
                    // DPNS contests — Refresh must reach the network,
                    // not just re-read the store.
                    self.reload();
                    inner = self.refresh_from_network();
                }
                ui.add_space(8.0);
                // Gate re-entry into the load form while a load is in flight
                //, and surface a spinner so the wait is visible.
                if self.pending_load.is_some() {
                    ui.spinner();
                    ui.add_enabled(false, egui::Button::new("Loading…"));
                } else if ComponentStyles::add_toolbar_button(ui, "+ Load", network_accent)
                    .clicked()
                {
                    self.view = MasternodesView::Load(self.new_load_form());
                }
            });
        });
        ui.add_space(8.0);

        if self.nodes.is_empty() {
            inner |= self.render_empty_state(ui);
        } else {
            inner |= self.render_card_grid(ui);
        }
        inner
    }

    /// Build the network re-fetch dispatched by the list Refresh button:
    /// one `RefreshIdentity` per loaded node, plus a DPNS contests
    /// re-query so vote counts refresh too. Returns `None` when no node is
    /// loaded (nothing to refresh).
    fn refresh_from_network(&self) -> AppAction {
        let identities = self
            .app_context
            .load_local_masternode_identities()
            .unwrap_or_default();
        if identities.is_empty() {
            return AppAction::None;
        }
        let mut tasks: Vec<BackendTask> = identities
            .into_iter()
            .map(|qi| BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qi)))
            .collect();
        tasks.push(BackendTask::ContestedResourceTask(
            ContestedResourceTask::QueryDPNSContests,
        ));
        AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Concurrent)
    }

    /// Render the load form; map its outcome to a backend load task and return
    /// to the list on cancel or submit.
    fn render_load_view(&mut self, ui: &mut egui::Ui) -> AppAction {
        let dev_mode = self.app_context.user_role().at_least(UserRole::Power);
        let submitting = self.pending_load.is_some();
        let outcome = match &mut self.view {
            MasternodesView::Load(form) => form.show(ui, dev_mode, submitting),
            _ => return AppAction::None,
        };
        self.apply_load_outcome(outcome)
    }

    /// Apply one frame's load-form outcome: cancel returns to the list, submit
    /// dispatches the backend load.
    fn apply_load_outcome(&mut self, outcome: LoadFormOutcome) -> AppAction {
        match outcome {
            LoadFormOutcome::None => AppAction::None,
            LoadFormOutcome::Cancel => {
                // Cancel dismisses the form, not the load: a dispatched load has
                // no cooperative cancellation and keeps running. Hold the gate so
                // reopening the form cannot submit that same node a second time;
                // it clears once the load is seen to finish.
                self.view = MasternodesView::List;
                AppAction::None
            }
            LoadFormOutcome::Submit(mut input) => {
                // Gate on the identity actually submitted, not on what the form
                // shows later: every field but the Load button stays editable
                // while the load runs.
                //
                // Mark the dispatch before the task runs: a load is outstanding
                // from here, and until its task claims the identity nothing else
                // records that. Keep the form open with its fields intact
                // meanwhile — `reconcile_pending_load` settles it against the phase
                // the task reports, closing it only on a load that fully applied.
                //
                // No token means another caller is already loading this node: that
                // load owns the record, so gate on nothing and let the dispatch come
                // back with `IdentityLoadInProgress` for the banner to explain.
                self.pending_load =
                    decode_identity_id(&input.identity_id_input)
                        .ok()
                        .and_then(|identity_id| {
                            self.app_context
                                .mark_identity_load_submitted(identity_id)
                                .map(|token| PendingLoad { identity_id, token })
                        });
                // Send the token with the load, so the task claims the record this
                // screen is waiting on — and so no other load can claim it.
                input.load_token = self.pending_load.map(|pending| pending.token);
                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::LoadIdentity(
                    *input,
                )))
            }
        }
    }
}

impl ScreenLike for MasternodesScreen {
    fn refresh(&mut self) {
        self.reload();
    }

    fn refresh_on_arrival(&mut self) {
        self.reload();
        self.reconcile_pending_load();
        // An open detail view was built from a record another screen may have
        // written meanwhile — a restore run from the Key Info screen this view
        // pushed is exactly that case, since the pushed screen receives the
        // result and this one never hears about it. Re-read the node and re-arm
        // its recovery check, so the page never keeps offering keys that are
        // already back.
        if let MasternodesView::Detail(detail) = &mut self.view {
            detail.refresh_from_store();
        }
    }

    fn reset_to_root_view(&mut self) {
        if !matches!(self.view, MasternodesView::Load(_)) {
            self.view = MasternodesView::List;
        }
    }

    /// Drop every secret the open view holds — the load form's keys and
    /// encryption password, the detail view's unsubmitted voting key. This screen
    /// is a root screen: it lives for the whole process, so nothing else would
    /// ever zeroize them, and a failed load deliberately keeps its form (and its
    /// fields) for a corrected resubmit.
    ///
    /// Unconditional, in-flight load or not. The load's own copy of the submitted
    /// input already travelled with the task, and a load that fails while the user
    /// is elsewhere is exactly the case that would otherwise strand plaintext keys
    /// on a screen nobody is looking at. What survives is what is tedious to
    /// re-enter and secret to nobody: ProTxHash, alias, node type.
    fn on_leave(&mut self) {
        match &mut self.view {
            MasternodesView::Load(form) => form.clear_secrets(),
            MasternodesView::Detail(detail) => detail.clear_secrets(),
            MasternodesView::List => {}
        }
    }

    fn display_task_result(&mut self, result: crate::backend_task::BackendTaskSuccessResult) {
        match result {
            // A recovery preview changed nothing in the store, so it is routed
            // into the open detail view instead of reloading and re-opening it.
            // Re-opening would rebuild the view, re-dispatch its check, and
            // never settle.
            BackendTaskSuccessResult::LegacyRecoveryCandidates { .. } => {
                let ctx = self.app_context.egui_ctx().clone();
                if let MasternodesView::Detail(detail) = &mut self.view {
                    detail.absorb_recovery_result(&ctx, &result);
                }
                return;
            }
            // A restore did change the store. Confirm it, then fall through to
            // the reload, whose tail re-opens the detail view for this node —
            // rebuilding it re-arms the check, which now finds nothing stranded
            // and retires the offer.
            //
            // A restore for an identity this page is not showing lands here
            // whenever this screen happens to be the visible one, so it is
            // dropped rather than reported: it changed nothing on screen, and
            // its "your keys are back" belongs to whoever asked for it.
            BackendTaskSuccessResult::LegacyRecoveryCompleted {
                identity_id,
                ref applied,
                ..
            } => {
                if !self.shows_node(identity_id) {
                    return;
                }
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    completion_message(!applied.is_empty()),
                    MessageType::Success,
                );
            }
            _ => {}
        }
        self.reload();
        // Settle the submitted load against the phase its task reported. This
        // screen also receives detail-view results (voting, RefreshIdentity) and
        // another screen's load result, so the gate must never turn on "a result
        // arrived" — only on this load's own reported outcome.
        self.reconcile_pending_load();
        // if a detail view is open, its own backend task (voting, an
        // Add-voting-key merge, a RefreshIdentity) just updated the store.
        // Re-open the detail view for that node so the on-screen view reflects
        // the fresh data instead of the stale clone captured at open time.
        if let MasternodesView::Detail(detail) = &self.view {
            let node_id = detail.node_id();
            self.open_detail(node_id);
        }
    }

    fn display_task_error(&mut self, _error: &crate::backend_task::error::TaskError) -> bool {
        // A failing load reports `Failed` before its error reaches the UI, so
        // settling here re-enables the still-open form's submit button (the Load
        // view is untouched, so every entered field survives for correction).
        // An error from some other task — a detail-view vote, a refresh — leaves
        // this load's phase outstanding and the gate held. Let the global banner
        // render the error (return false).
        self.reconcile_pending_load();
        false
    }

    /// End the open detail view's recovery operation only when the failure is
    /// that operation's own. A failed restore returns to its offer so the user
    /// can correct a mistyped identity password and press Restore again; any
    /// other task's error — which reaches this screen simply because it is
    /// visible — must leave a running restore in flight.
    fn display_backend_task_error(
        &mut self,
        context: &crate::backend_task::BackendTaskContext,
        _error: &crate::backend_task::error::TaskError,
    ) {
        if let MasternodesView::Detail(detail) = &mut self.view {
            detail.absorb_recovery_error(context);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let (mut action, effect) =
            add_top_panel_with_global_nav_capturing(ui, &self.app_context, self.nav_spec(), vec![]);
        self.apply_nav_effect(effect);

        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenMasternodes);

        let network_accent =
            DashColors::network_accent(self.app_context.network, ui.style().visuals.dark_mode);

        action |= island_central_panel(ui, |ui| {
            ui.set_min_width(ui.available_width());
            match self.view {
                MasternodesView::Load(_) => self.render_load_view(ui),
                MasternodesView::Detail(_) => self.render_detail_view(ui, network_accent),
                MasternodesView::List => self.render_list_view(ui, network_accent),
            }
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::backend_task::identity::IdentityInputToLoad;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::qualified_identity::QualifiedIdentity;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;

    /// Build an offline, wallet-backend-wired `AppContext` (no network I/O).
    async fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        (ctx, temp_dir)
    }

    fn seed_masternode(ctx: &Arc<AppContext>, byte: u8, alias: Option<&str>) {
        let pv = PlatformVersion::latest();
        let identity = Identity::create_basic_identity(Identifier::from([byte; 32]), pv)
            .expect("basic identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: alias.map(str::to_owned),
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::PendingCreation,
            network: ctx.network(),
        };
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("seed masternode");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nodes_are_sorted_case_insensitively_by_display_heading() {
        let (ctx, _tmp) = offline_ctx().await;
        seed_masternode(&ctx, 0x11, Some("Zulu"));
        seed_masternode(&ctx, 0x22, Some("alpha"));
        seed_masternode(&ctx, 0x44, None);

        let fallback_heading = shorten_id(&Identifier::from([0x44; 32]).to_string(Encoding::Hex));
        let screen = MasternodesScreen::new(&ctx);

        assert_eq!(
            screen.node_headings_for_test(),
            vec![fallback_heading, "alpha".to_string(), "Zulu".to_string()]
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// The list Refresh builds one `RefreshIdentity` per loaded node plus a
    /// single trailing `QueryDPNSContests`, and yields `None` when no node is
    /// loaded (nothing to refresh).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_from_network_builds_per_node_refresh_plus_contest_requery() {
        let (ctx, _tmp) = offline_ctx().await;

        // No nodes loaded → nothing to refresh.
        let screen = MasternodesScreen::new(&ctx);
        assert!(
            matches!(screen.refresh_from_network(), AppAction::None),
            "an empty node list must produce no refresh task"
        );

        // Two nodes loaded → two RefreshIdentity + one trailing QueryDPNSContests.
        seed_masternode(&ctx, 0x11, None);
        seed_masternode(&ctx, 0x22, None);
        let screen = MasternodesScreen::new(&ctx);
        let AppAction::BackendTasks(tasks, mode) = screen.refresh_from_network() else {
            panic!("expected BackendTasks");
        };
        assert!(matches!(mode, BackendTasksExecutionMode::Concurrent));
        assert_eq!(tasks.len(), 3, "two node refreshes + one contest re-query");
        let refreshes = tasks
            .iter()
            .filter(|t| {
                matches!(
                    t,
                    BackendTask::IdentityTask(IdentityTask::RefreshIdentity(_))
                )
            })
            .count();
        assert_eq!(refreshes, 2, "one RefreshIdentity per loaded node");
        assert!(
            matches!(
                tasks.last(),
                Some(BackendTask::ContestedResourceTask(
                    ContestedResourceTask::QueryDPNSContests
                ))
            ),
            "the contest re-query must be the trailing task",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// FR-GLOBAL-NAV-3 — the node pill is two-way bound with the page: opening a
    /// node (what a card click does) puts it on the pill, and picking a node
    /// from the pill opens its detail view. The pill lists every loaded node.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn node_pill_is_two_way_bound_with_the_detail_view() {
        let (ctx, _tmp) = offline_ctx().await;
        seed_masternode(&ctx, 0x11, None);
        seed_masternode(&ctx, 0x22, None);
        let mut screen = MasternodesScreen::new(&ctx);

        // On the grid, no node is in view: the pill offers both, selects none.
        let (scope, consumption) = screen
            .nav_spec()
            .identity_pill()
            .cloned()
            .expect("node pill");
        assert!(consumption.is_consumed(), "the node pill is interactive");
        assert!(scope.is_page_scoped(), "a node is never the app identity");
        assert_eq!(scope.page_scoped_selection(), None);

        // Grid → pill: opening a node's detail view puts it on the pill.
        let node = Identifier::from([0x22; 32]);
        screen.open_detail(node);
        assert!(matches!(screen.view, MasternodesView::Detail(_)));
        assert_eq!(
            screen
                .nav_spec()
                .identity_pill()
                .expect("node pill")
                .0
                .page_scoped_selection(),
            Some(node),
            "the node in view must show on the pill",
        );

        // Pill → grid: picking the other node opens that node's detail view.
        let other = Identifier::from([0x11; 32]);
        screen.apply_nav_effect(GlobalNavEffect::SelectPageObject(other));
        let MasternodesView::Detail(detail) = &screen.view else {
            panic!("picking a node from the pill must open its detail view");
        };
        assert_eq!(detail.node_id(), other);

        // A wallet switch is the shared applier's business — it must not move
        // the page off the node in view.
        screen.apply_nav_effect(GlobalNavEffect::SwitchWallet([0u8; 32]));
        assert_eq!(screen.selected_node_id(), Some(other));

        // Leaving the detail view clears the pill's selection.
        screen.view = MasternodesView::List;
        assert_eq!(screen.selected_node_id(), None);

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A restore run from the pushed Key Info screen never reaches this screen —
    /// the pushed screen is on top, so it takes the result. Coming back must
    /// therefore re-read the node and re-arm its check, or the page keeps
    /// offering keys that are already back and keeps warning about a voting key
    /// it now holds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_re_reads_the_open_node_and_re_arms_its_recovery_offer() {
        use crate::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor, RecoveryPlan};
        use dash_sdk::platform::IdentityPublicKey;

        let (ctx, _tmp) = offline_ctx().await;
        let node = Identifier::from([0x33; 32]);
        seed_masternode(&ctx, 0x33, None);
        let mut screen = MasternodesScreen::new(&ctx);
        screen.open_detail(node);

        // The offer the node page was showing before the user opened a key.
        let MasternodesView::Detail(detail) = &mut screen.view else {
            panic!("the detail view must be open");
        };
        detail.set_recovery_plan(
            node,
            RecoveryPlan {
                items: vec![RecoveryItemDescriptor {
                    item: RecoveryItem::VoterAssociation,
                    purpose: None,
                }],
                excluded: vec![],
            },
        );
        assert!(detail.has_recovery_offer_for_test());
        assert!(
            !detail.key_presence_for_test().voting,
            "the node starts with no voting key",
        );

        // What the pushed Key Info screen's restore wrote while this screen was
        // not the one receiving results.
        let mut restored = ctx
            .get_local_qualified_identity(&node)
            .expect("read the node")
            .expect("node stored");
        restored.associated_voter_identity = Some((
            Identity::create_basic_identity(
                Identifier::from([0x44; 32]),
                PlatformVersion::latest(),
            )
            .expect("voter identity"),
            IdentityPublicKey::V0(
                dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0 {
                    id: 0,
                    purpose: dash_sdk::dpp::identity::Purpose::VOTING,
                    security_level: dash_sdk::dpp::identity::SecurityLevel::HIGH,
                    contract_bounds: None,
                    key_type: dash_sdk::dpp::identity::KeyType::ECDSA_HASH160,
                    read_only: false,
                    data: dash_sdk::dpp::platform_value::BinaryData::new(vec![7u8; 20]),
                    disabled_at: None,
                },
            ),
        ));
        ctx.update_local_qualified_identity(&restored)
            .expect("the restore's write");

        screen.refresh_on_arrival();

        let MasternodesView::Detail(detail) = &screen.view else {
            panic!("the detail view must still be open");
        };
        assert!(
            !detail.has_recovery_offer_for_test(),
            "the stale offer must be retired on arrival, not re-shown",
        );
        assert!(
            detail.key_presence_for_test().voting,
            "the node page must show the voting key it now holds",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A restore for an identity this page is not showing lands here whenever
    /// this screen happens to be the visible one — the Key Info screen that
    /// dispatched it may have been dismissed long since. Acting on it would
    /// confirm someone else's restore over the node in view and rebuild that
    /// node's detail from a store nothing changed in.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_completion_for_a_node_not_on_screen_is_ignored() {
        use crate::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor, RecoveryPlan};

        let (ctx, _tmp) = offline_ctx().await;
        let node = Identifier::from([0x33; 32]);
        seed_masternode(&ctx, 0x33, None);
        let mut screen = MasternodesScreen::new(&ctx);
        screen.open_detail(node);

        // The offer the node in view is showing, which its own re-open would
        // retire — so a wrongly-attributed reload is observable.
        let MasternodesView::Detail(detail) = &mut screen.view else {
            panic!("the detail view must be open");
        };
        detail.set_recovery_plan(
            node,
            RecoveryPlan {
                items: vec![RecoveryItemDescriptor {
                    item: RecoveryItem::VoterAssociation,
                    purpose: None,
                }],
                excluded: vec![],
            },
        );

        screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCompleted {
            identity_id: Identifier::from([0x77; 32]),
            applied: vec![],
            skipped_stale: vec![],
            excluded: vec![],
        });

        let MasternodesView::Detail(detail) = &screen.view else {
            panic!("the detail view must still be open");
        };
        assert_eq!(detail.node_id(), node);
        assert!(
            detail.has_recovery_offer_for_test(),
            "another identity's completion must leave this node's offer alone",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: this screen used to end the open node's restore on ANY task
    /// error that reached it — a vote, a refresh, another node's restore. The
    /// original task still held the identity, so re-enabling Restore only led
    /// the user to an "already in progress" error. Only this node's own
    /// recovery failure may return its offer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_this_nodes_own_recovery_failure_ends_its_restore() {
        use crate::backend_task::BackendTaskContext;
        use crate::backend_task::error::TaskError;
        use crate::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor, RecoveryPlan};

        let (ctx, _tmp) = offline_ctx().await;
        let node = Identifier::from([0x33; 32]);
        seed_masternode(&ctx, 0x33, None);
        let mut screen = MasternodesScreen::new(&ctx);
        screen.open_detail(node);

        let MasternodesView::Detail(detail) = &mut screen.view else {
            panic!("the detail view must be open");
        };
        detail.set_recovery_plan(
            node,
            RecoveryPlan {
                items: vec![RecoveryItemDescriptor {
                    item: RecoveryItem::VoterAssociation,
                    purpose: None,
                }],
                excluded: vec![],
            },
        );
        assert!(
            detail.start_recovery_restore_for_test(),
            "the offer must be restorable, or this proves nothing",
        );

        let error = TaskError::IdentityNotFoundLocally;
        for unrelated in [
            BackendTaskContext::Other,
            BackendTaskContext::LegacyRecoveryRestore(Identifier::from([0x77; 32])),
        ] {
            screen.display_backend_task_error(&unrelated, &error);
            screen.display_task_error(&error);
            let MasternodesView::Detail(detail) = &screen.view else {
                panic!("the detail view must still be open");
            };
            assert!(
                detail.is_restoring_for_test(),
                "{unrelated:?} is not this node's restore, so it must stay in flight",
            );
        }

        screen.display_backend_task_error(&BackendTaskContext::LegacyRecoveryRestore(node), &error);
        let MasternodesView::Detail(detail) = &screen.view else {
            panic!("the detail view must still be open");
        };
        assert!(
            !detail.is_restoring_for_test(),
            "this node's own restore failure must return the offer so it can be retried",
        );
        assert!(
            detail.has_recovery_offer_for_test(),
            "and the offer must survive for the retry",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    impl MasternodesScreen {
        /// The identity of the load being gated on, if any.
        fn pending_identity(&self) -> Option<Identifier> {
            self.pending_load.map(|pending| pending.identity_id)
        }
    }

    /// Drive the real submit path: open the form on `target` and apply the
    /// outcome a Load click produces, exactly as `render_load_view` would.
    fn submit_load(screen: &mut MasternodesScreen, target: Identifier) {
        let mut form = screen.new_load_form();
        form.set_pro_tx_hash_for_test(target.to_string(Encoding::Base58));
        let outcome = form.submit_for_test();
        screen.view = MasternodesView::Load(form);
        screen.apply_load_outcome(outcome);
    }

    /// Claim the identity for the load this screen dispatched, exactly as its own
    /// task does: under the token the screen is waiting on. A claim taken under any
    /// other token is a *foreign* load — the registry rejects it while this one is
    /// outstanding, and it could never report into this screen's record.
    fn claim_dispatched_load(
        ctx: &Arc<AppContext>,
        screen: &MasternodesScreen,
    ) -> crate::context::identity_load_registry::IdentityLoadGuard {
        let pending = screen.pending_load.expect("the screen dispatched a load");
        ctx.begin_identity_load(pending.identity_id, Some(pending.token))
            .expect("claim the load")
    }

    /// Point the open form at another node — every field but the Load button
    /// stays editable while a load runs.
    fn retarget_open_form(screen: &mut MasternodesScreen, target: Identifier) {
        let MasternodesView::Load(form) = &mut screen.view else {
            panic!("the load form must be open");
        };
        form.set_pro_tx_hash_for_test(target.to_string(Encoding::Base58));
    }

    fn masternode_identity(ctx: &Arc<AppContext>, id: Identifier) -> QualifiedIdentity {
        let identity =
            Identity::create_basic_identity(id, PlatformVersion::latest()).expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::PendingCreation,
            network: ctx.network(),
        }
    }

    /// Submitting gates on the identity that was submitted, and a failed load
    /// keeps the form open (fields intact) and re-enables submit; only a
    /// successful load closes the form and returns to the list.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn load_error_keeps_form_open_success_closes_it() {
        use crate::backend_task::BackendTaskSuccessResult;
        use crate::backend_task::error::TaskError;

        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x33; 32]);
        submit_load(&mut screen, target);
        assert_eq!(
            screen.pending_identity(),
            Some(target),
            "the gate must record the submitted identity"
        );

        // The task ran and failed; it reported `Failed` before its error reached
        // the UI. The screen defers to the global banner, keeps the form open, and
        // re-enables submit.
        drop(claim_dispatched_load(&ctx, &screen));
        let handled = screen.display_task_error(&TaskError::MalformedProTxHash {
            input: "not-a-hash".to_string(),
        });
        assert!(
            !handled,
            "the global banner renders the error, not the screen"
        );
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "the form must stay open on error so fields can be corrected"
        );
        assert!(
            screen.pending_load.is_none(),
            "submit must re-enable after a failed load"
        );

        screen.reset_to_root_view();
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "root navigation must preserve a failed load form for correction"
        );

        // Resubmit, then succeed: the form closes and drops back to the list.
        submit_load(&mut screen, target);
        claim_dispatched_load(&ctx, &screen).loaded();
        screen.display_task_result(BackendTaskSuccessResult::LoadedIdentity(
            masternode_identity(&ctx, target),
        ));
        assert!(
            matches!(screen.view, MasternodesView::List),
            "a successful load must close the form and return to the list"
        );
        assert!(
            screen.pending_load.is_none(),
            "the in-flight gate must clear on success"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Another screen's load result must not release this screen's gate: it
    /// belongs to a different identity, and this screen's own load is still
    /// running.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn another_identitys_load_result_leaves_the_gate_locked() {
        use crate::backend_task::BackendTaskSuccessResult;

        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x33; 32]);
        submit_load(&mut screen, target);

        screen.display_task_result(BackendTaskSuccessResult::LoadedIdentity(
            masternode_identity(&ctx, Identifier::from([0x99; 32])),
        ));

        assert_eq!(
            screen.pending_identity(),
            Some(target),
            "only the submitted identity's own result releases the gate"
        );
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "the form must stay open while its own load runs"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: a load whose result was delivered to another visible screen
    /// (task results reach only the visible screen) must still close the form on
    /// return to the tab. The load is no longer running and its node is in the
    /// store, so the form closes and the gate clears — without ever routing the
    /// result through this screen's callbacks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_reconciles_form_when_load_finished_while_away() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x33; 32]);
        submit_load(&mut screen, target);

        // The load fully applied while another screen was visible: the node landed
        // in the store and the task reported success, but this screen's
        // `display_task_result` never fired.
        let running = claim_dispatched_load(&ctx, &screen);
        seed_masternode(&ctx, 0x33, None);
        running.loaded();

        screen.refresh_on_arrival();
        assert!(
            matches!(screen.view, MasternodesView::List),
            "a load that finished while away must close the form on return"
        );
        assert!(
            screen.pending_load.is_none(),
            "the gate must clear once the loaded node is in the store"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: while the submitted load is still running, revisiting the tab
    /// must NOT unlock the form — a second concurrent load of the same node would
    /// race the first past the duplicate check.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_keeps_form_locked_while_load_still_pending() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x44; 32]);
        submit_load(&mut screen, target);
        // The load task is running: it holds the identity's claim.
        let _running = claim_dispatched_load(&ctx, &screen);

        screen.refresh_on_arrival();
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "a still-pending load must keep the form open"
        );
        assert_eq!(
            screen.pending_identity(),
            Some(target),
            "the gate must stay locked so a second concurrent load cannot be submitted"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: reconciliation must track the identity that was submitted, not
    /// whatever the still-editable form points at now. Retargeting the open form
    /// at an already-loaded node must not read as "the submitted load finished" —
    /// that would unlock the form and let the submitted node be loaded a second
    /// time, concurrently.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_tracks_the_submitted_load_not_the_edited_form() {
        let (ctx, _tmp) = offline_ctx().await;

        // `other` is already loaded; `submitted` is the node whose load is running.
        seed_masternode(&ctx, 0x66, None);
        let other = Identifier::from([0x66; 32]);
        let submitted = Identifier::from([0x55; 32]);
        let mut screen = MasternodesScreen::new(&ctx);

        submit_load(&mut screen, submitted);
        let _running = claim_dispatched_load(&ctx, &screen);
        retarget_open_form(&mut screen, other);

        screen.refresh_on_arrival();

        assert_eq!(
            screen.pending_identity(),
            Some(submitted),
            "the submitted load is still running: the gate must stay locked"
        );
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "retargeting the form must not close it"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: Cancel dismisses the form, not the backend task — loads have no
    /// cooperative cancellation, so the dispatched load keeps running. Releasing
    /// the gate would let the user reopen the form and submit the same node again,
    /// racing the first load's duplicate check.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_keeps_the_gate_while_the_submitted_load_still_runs() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x77; 32]);
        submit_load(&mut screen, target);
        let _running = claim_dispatched_load(&ctx, &screen);

        screen.apply_load_outcome(LoadFormOutcome::Cancel);

        assert!(
            matches!(screen.view, MasternodesView::List),
            "Cancel must dismiss the form"
        );
        assert_eq!(
            screen.pending_identity(),
            Some(target),
            "Cancel does not cancel the task: the gate must hold until the load finishes"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A load that FAILED while another screen was visible leaves no trace in the
    /// store, so the store alone cannot tell it from a load still in progress. The
    /// phase the task reports can: the load is finished, so the gate releases and
    /// the form stays open with its fields intact for a corrected resubmit.
    /// Without this, the load entry points would stay disabled for the rest of the
    /// session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_releases_the_gate_when_the_load_failed_while_away() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x88; 32]);
        submit_load(&mut screen, target);

        // The load ran and failed while away; the node never landed.
        drop(claim_dispatched_load(&ctx, &screen));
        screen.refresh_on_arrival();

        assert!(
            screen.pending_load.is_none(),
            "a finished load must release the gate, whichever screen saw its error"
        );
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "the form stays open with its fields intact so the user can resubmit"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: a dispatched load is outstanding from the moment it is
    /// submitted — the task only reaches its claim after it starts and validates
    /// its input. Arriving inside that window must not read "no claim yet" as
    /// "finished": releasing the gate there strands the real load, whose success
    /// can then no longer close the form.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_holds_the_gate_before_the_task_claims_the_load() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0xa1; 32]);
        submit_load(&mut screen, target);

        // The task has been dispatched but has not reached its claim yet.
        screen.refresh_on_arrival();

        assert_eq!(
            screen.pending_identity(),
            Some(target),
            "a submitted load is outstanding before its task claims it"
        );
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "the form must stay open while the submitted load is outstanding"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: a terminated load is not a successful one. `load_identity`
    /// inserts the node BEFORE sealing its keys, so a seal failure leaves the node
    /// in the store while the load errored. Treating "the node is present" as
    /// success closes the form and discards the user's retry state even though the
    /// keys may be unprotected or partially protected.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_does_not_read_a_persisted_node_as_a_successful_load() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0xb2; 32]);
        submit_load(&mut screen, target);

        // The task ran, inserted the node, then failed while sealing its keys.
        let running = claim_dispatched_load(&ctx, &screen);
        seed_masternode(&ctx, 0xb2, None);
        drop(running);

        screen.refresh_on_arrival();

        assert!(
            screen.pending_load.is_none(),
            "a finished load must release the gate"
        );
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "a failed load keeps its form open for a retry, even though the node was persisted"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Open the load form on `target` with every secret field filled.
    fn open_form_with_secrets(screen: &mut MasternodesScreen, target: Identifier, secret: &str) {
        let mut form = screen.new_load_form();
        form.set_pro_tx_hash_for_test(target.to_string(Encoding::Base58));
        form.set_secrets_for_test(secret);
        screen.view = MasternodesView::Load(form);
    }

    /// What the open form would submit right now — the values it still holds.
    fn form_input(screen: &MasternodesScreen) -> Box<IdentityInputToLoad> {
        let MasternodesView::Load(form) = &screen.view else {
            panic!("the load form must still be open");
        };
        let LoadFormOutcome::Submit(input) = form.submit_for_test() else {
            panic!("a form holding a ProTxHash must still submit");
        };
        input
    }

    fn assert_holds_no_secrets(screen: &MasternodesScreen) {
        let input = form_input(screen);
        assert!(
            input.voting_private_key_input.is_blank(),
            "the voting key must not outlive the tab"
        );
        assert!(
            input.owner_private_key_input.is_blank(),
            "the owner key must not outlive the tab"
        );
        assert!(
            input.payout_address_private_key_input.is_blank(),
            "the payout key must not outlive the tab"
        );
        assert!(
            input.encryption_password.is_none(),
            "the encryption password must not outlive the tab"
        );
    }

    /// SEC — the Masternodes tab is a root screen: it outlives navigation. The
    /// plaintext V/O/P keys and the at-load encryption password entered into its
    /// load form must not stay resident once the user leaves the tab. The fields
    /// that are tedious to re-enter and carry no secret — ProTxHash, alias, node
    /// type — survive, so the load can be resumed on return.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn leaving_the_tab_zeroizes_the_load_forms_secrets() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0xc3; 32]);
        open_form_with_secrets(&mut screen, target, "wif");

        screen.on_leave();

        assert_holds_no_secrets(&screen);
        assert_eq!(
            form_input(&screen).identity_id_input,
            target.to_string(Encoding::Base58),
            "the non-secret fields survive so the load can be resumed",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// SEC — the reported case: the user submits, navigates away while the load
    /// runs, and the load fails behind their back. The form is kept open for a
    /// corrected resubmit, so nothing else would ever drop its secrets. Leaving
    /// clears them even with the load still outstanding — the gate is unaffected,
    /// and the backend already holds its own copy of the submitted input.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn leaving_the_tab_zeroizes_secrets_of_an_outstanding_load() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0xc4; 32]);
        open_form_with_secrets(&mut screen, target, "wif");
        let outcome = form_input(&screen);
        screen.apply_load_outcome(LoadFormOutcome::Submit(outcome));
        let _running = claim_dispatched_load(&ctx, &screen);

        screen.on_leave();

        assert_holds_no_secrets(&screen);
        assert_eq!(
            screen.pending_identity(),
            Some(target),
            "leaving the tab does not cancel the load: the gate must still hold",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// SEC — the detail view's in-place `Add voting key` prompt holds a plaintext
    /// WIF too, and lives in the very same root screen. A prompt left filled but
    /// unsubmitted must not survive the user leaving the tab.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn leaving_the_tab_discards_an_unsubmitted_voting_key() {
        let (ctx, _tmp) = offline_ctx().await;
        seed_masternode(&ctx, 0xc5, None);
        let mut screen = MasternodesScreen::new(&ctx);
        screen.open_detail(Identifier::from([0xc5; 32]));

        let MasternodesView::Detail(detail) = &mut screen.view else {
            panic!("the detail view must be open");
        };
        detail.set_voter_key_prompt_for_test("voter-wif");

        screen.on_leave();

        let MasternodesView::Detail(detail) = &screen.view else {
            panic!("leaving must not discard the detail view");
        };
        assert!(
            !detail.has_voter_key_prompt_for_test(),
            "an unsubmitted voting key must not outlive the tab"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A node with no detail view open and no nodes at all both resolve to "no
    /// selection" — the pill falls back to its placeholder rather than naming a
    /// node the page is not showing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_page_offers_no_nodes_on_the_pill() {
        let (ctx, _tmp) = offline_ctx().await;
        let screen = MasternodesScreen::new(&ctx);

        let (scope, _) = screen
            .nav_spec()
            .identity_pill()
            .cloned()
            .expect("node pill");
        assert_eq!(scope.page_scoped_selection(), None);

        ctx.wallet_backend().expect("backend").shutdown().await;
    }
}
