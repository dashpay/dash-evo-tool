//! The Masternodes list root screen.
//!
//! B3 lands the empty state (FR-2) and the card grid (FR-3) on top of the B2
//! scaffold: the global-nav header, the left nav rail, and an island content
//! panel. The empty state reuses the identity onboarding card pattern; the grid
//! reuses the identity-picker card visual language via [`MasternodeCard`]. A
//! top-right Refresh button (FR-7) re-reads the local node list and its DPNS
//! contest summaries.
//!
//! The page-scoped masternode pill is wired in B7; card clicks open the detail
//! view built in B5a.

use std::sync::Arc;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, RichText};

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::contested_name::MasternodeContestSummary;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, MasternodeKeyPresence};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_wallet_only_spec};
use crate::ui::identity::identity_pill::shorten_id;
use crate::ui::identity::picker::compute_column_count;
use crate::ui::masternodes::card::MasternodeCard;
use crate::ui::masternodes::detail_screen::{DetailOutcome, MasternodeDetailView};
use crate::ui::masternodes::load_form::{LoadFormOutcome, MasternodeLoadForm};
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
            if ComponentStyles::add_primary_button(ui, "Load a masternode").clicked() {
                self.view = MasternodesView::Load(Box::default());
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
                    self.reload();
                }
                ui.add_space(8.0);
                if ComponentStyles::add_toolbar_button(ui, "+ Load", network_accent).clicked() {
                    self.view = MasternodesView::Load(Box::default());
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

    /// Render the load form; map its outcome to a backend load task and return
    /// to the list on cancel or submit.
    fn render_load_view(&mut self, ui: &mut egui::Ui) -> AppAction {
        let outcome = match &mut self.view {
            MasternodesView::Load(form) => form.show(ui),
            _ => return AppAction::None,
        };
        match outcome {
            LoadFormOutcome::None => AppAction::None,
            LoadFormOutcome::Cancel => {
                self.view = MasternodesView::List;
                AppAction::None
            }
            LoadFormOutcome::Submit(input) => {
                self.view = MasternodesView::List;
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
    }

    fn display_task_result(&mut self, _result: crate::backend_task::BackendTaskSuccessResult) {
        // A completed load (or any task routed here) may have added a node —
        // re-read the cached list so the new card appears.
        self.reload();
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        // TODO(B7): replace the wallet-only spec with the page-scoped masternode
        // pill (IdentityPillScope::PageScopedObject).
        let mut action = add_top_panel_with_global_nav(
            ui,
            &self.app_context,
            subdued_wallet_only_spec("Masternodes", RootScreenType::RootScreenMasternodes),
            vec![],
        );

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
