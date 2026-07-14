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
use crate::backend_task::BackendTask;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::contested_name::MasternodeContestSummary;
use crate::model::masternode_input::decode_identity_id;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, MasternodeKeyPresence};
use crate::model::user_role::UserRole;
use crate::ui::components::global_nav_switcher::GlobalNavEffect;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel_with_global_nav_capturing;
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::identity::picker::compute_column_count;
use crate::ui::masternodes::card::MasternodeCard;
use crate::ui::masternodes::detail_screen::{DetailOutcome, MasternodeDetailView};
use crate::ui::masternodes::load_form::{LoadFormOutcome, MasternodeLoadForm};
use crate::ui::state::global_nav::PageNavSpec;
use crate::ui::state::masternodes_view::{masternodes_page_nav_spec, node_pill_item};
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::{RootScreenType, ScreenLike};

/// Minimum horizontal gap between cards in the grid (matches the identity
/// picker grid).
const GRID_GAP: f32 = 16.0;

/// Pre-resolved display data for one masternode/evonode card. Computed at
/// reload time so the per-frame render never touches the database.
struct NodeCardData {
    node_id: Identifier,
    node_id_short: String,
    alias: Option<String>,
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

/// Root screen for the Masternodes section.
pub struct MasternodesScreen {
    pub app_context: Arc<AppContext>,
    /// Cached card data for the active network, refreshed on arrival, on
    /// `refresh`, and on the Refresh button.
    nodes: Vec<NodeCardData>,
    /// The active sub-view (list / load / detail).
    view: MasternodesView,
    /// The identity whose load this screen dispatched and has not yet seen
    /// finish, captured from the ProTxHash that was *submitted* — never re-read
    /// from the form, whose fields stay editable while the load runs. Gates the
    /// load entry points (`+ Load`, the empty-state CTA, the form's submit
    /// button) so a load cannot be dispatched twice.
    ///
    /// `None` once the load is observed to finish — through its result or error,
    /// or, when those reached another screen, through
    /// [`reconcile_pending_load`](Self::reconcile_pending_load) on the next
    /// arrival. Also `None` when the submitted ProTxHash did not parse: the
    /// backend rejects such a load before touching the store, so there is nothing
    /// to gate.
    ///
    /// This gate is the UX layer — it keeps the buttons honest. The exclusion
    /// that actually protects the store is the load task's own claim on the
    /// identity, so a duplicate dispatch that slips through fails closed with
    /// [`TaskError::IdentityLoadInProgress`](crate::backend_task::error::TaskError::IdentityLoadInProgress)
    /// instead of racing.
    pending_load: Option<Identifier>,
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
                    node_type: qi.identity_type,
                    key_presence: qi.masternode_key_presence(),
                    contest_summary,
                    status: qi.status,
                }
            })
            .collect();
    }

    /// Reconcile a load whose result was delivered to another screen. Task
    /// results reach only the visible screen, so a load that finished while this
    /// tab was away never cleared the gate through
    /// `display_task_result`/`display_task_error`.
    ///
    /// The backend registry — not the store, and not the form — is the truth for
    /// "is it still running": a *failed* load leaves no trace in the store, so
    /// reading only the store cannot tell failure from a load still in progress
    /// and would keep the entry points disabled forever. While the submitted load
    /// runs the gate stays locked; once it is done, release it and close the form
    /// if the node actually landed. A load that failed keeps its form open with
    /// every field intact, ready for a corrected resubmit — the error was already
    /// shown as a global banner wherever it landed.
    fn reconcile_pending_load(&mut self) {
        let Some(target) = self.pending_load else {
            return;
        };
        if self.app_context.identity_load_in_flight(&target) {
            return;
        }
        self.pending_load = None;
        if matches!(self.view, MasternodesView::Load(_))
            && self.nodes.iter().any(|node| node.node_id == target)
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
                        .with_alias(node.alias.clone());
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

    /// The page's global-nav spec: an interactive wallet pill plus a node pill
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
        masternodes_page_nav_spec(items, self.selected_node_id())
    }

    /// Consume the global-nav effect this page is bound to: a node picked from
    /// the node pill opens its detail view. Every other effect (segment-1
    /// navigation, a wallet switch) is already applied by the shared applier.
    fn apply_nav_effect(&mut self, effect: GlobalNavEffect) {
        if let GlobalNavEffect::SelectPageObject(node_id) = effect {
            self.open_detail(node_id);
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
            LoadFormOutcome::Submit(input) => {
                // Gate on the identity actually submitted, not on what the form
                // shows later: every field but the Load button stays editable
                // while the load runs.
                //
                // Keep the form open with its fields intact meanwhile. On success
                // `display_task_result` closes it; on error `display_task_error`
                // re-enables submit so the user can correct one bad field and
                // resubmit — no full re-entry (QA follow-up).
                self.pending_load = decode_identity_id(&input.identity_id_input).ok();
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
    }

    fn display_task_result(&mut self, result: crate::backend_task::BackendTaskSuccessResult) {
        // Release the gate only on the result of the load this screen submitted.
        // It also receives detail-view results (voting, RefreshIdentity) and can
        // receive another screen's load — releasing on any of those would
        // re-enable `+ Load` while this screen's load is still running.
        if let crate::backend_task::BackendTaskSuccessResult::LoadedIdentity(loaded) = &result
            && self.pending_load == Some(loaded.identity.id())
        {
            self.pending_load = None;
            // The load succeeded — close the form and drop back to the list,
            // where the newly loaded node now shows as a card.
            if matches!(self.view, MasternodesView::Load(_)) {
                self.view = MasternodesView::List;
            }
        }
        self.reload();
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
        // Release the gate so the still-open form's submit button re-enables (the
        // Load view is untouched, so every entered field survives for
        // correction) — but only once the load it tracks has really finished. The
        // failing task drops its registry claim before its error reaches the UI,
        // so a claim that still stands belongs to a load that is still running,
        // and this error came from some other task (a detail-view vote, a
        // refresh). Let the global banner render the error (return false).
        if let Some(target) = self.pending_load
            && !self.app_context.identity_load_in_flight(&target)
        {
            self.pending_load = None;
        }
        false
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

    fn seed_masternode(ctx: &Arc<AppContext>, byte: u8) {
        let pv = PlatformVersion::latest();
        let identity = Identity::create_basic_identity(Identifier::from([byte; 32]), pv)
            .expect("basic identity");
        let qi = QualifiedIdentity {
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
        };
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("seed masternode");
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
        seed_masternode(&ctx, 0x11);
        seed_masternode(&ctx, 0x22);
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
        seed_masternode(&ctx, 0x11);
        seed_masternode(&ctx, 0x22);
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

    /// Drive the real submit path: open the form on `target` and apply the
    /// outcome a Load click produces, exactly as `render_load_view` would.
    fn submit_load(screen: &mut MasternodesScreen, target: Identifier) {
        let mut form = screen.new_load_form();
        form.set_pro_tx_hash_for_test(target.to_string(Encoding::Base58));
        let outcome = form.submit_for_test();
        screen.view = MasternodesView::Load(form);
        screen.apply_load_outcome(outcome);
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
            screen.pending_load,
            Some(target),
            "the gate must record the submitted identity"
        );

        // Error: the screen defers to the global banner, keeps the form open, and
        // re-enables submit by releasing the gate (the failing task released its
        // claim before the error reached the UI).
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

        // Resubmit, then succeed: the form closes and drops back to the list.
        submit_load(&mut screen, target);
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
            screen.pending_load,
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

        // The load completed while another screen was visible: the node landed in
        // the store, but this screen's `display_task_result` never fired.
        seed_masternode(&ctx, 0x33);

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
        let _running = ctx.begin_identity_load(target).expect("claim the load");

        screen.refresh_on_arrival();
        assert!(
            matches!(screen.view, MasternodesView::Load(_)),
            "a still-pending load must keep the form open"
        );
        assert_eq!(
            screen.pending_load,
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
        seed_masternode(&ctx, 0x66);
        let other = Identifier::from([0x66; 32]);
        let submitted = Identifier::from([0x55; 32]);
        let mut screen = MasternodesScreen::new(&ctx);

        submit_load(&mut screen, submitted);
        let _running = ctx.begin_identity_load(submitted).expect("claim the load");
        retarget_open_form(&mut screen, other);

        screen.refresh_on_arrival();

        assert_eq!(
            screen.pending_load,
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
        let _running = ctx.begin_identity_load(target).expect("claim the load");

        screen.apply_load_outcome(LoadFormOutcome::Cancel);

        assert!(
            matches!(screen.view, MasternodesView::List),
            "Cancel must dismiss the form"
        );
        assert_eq!(
            screen.pending_load,
            Some(target),
            "Cancel does not cancel the task: the gate must hold until the load finishes"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A load that FAILED while another screen was visible leaves no trace in the
    /// store, so the store alone cannot tell it from a load still in progress.
    /// The backend registry can: the task is gone, so the gate releases and the
    /// form stays open with its fields intact for a corrected resubmit. Without
    /// this, the load entry points would stay disabled for the rest of the
    /// session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn arrival_releases_the_gate_when_the_load_failed_while_away() {
        let (ctx, _tmp) = offline_ctx().await;
        let mut screen = MasternodesScreen::new(&ctx);

        let target = Identifier::from([0x88; 32]);
        submit_load(&mut screen, target);

        // The load failed while away: no claim is held, and the node never landed.
        assert!(!ctx.identity_load_in_flight(&target));
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
