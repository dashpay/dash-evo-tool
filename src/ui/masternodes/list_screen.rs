//! The Masternodes list root screen.
//!
//! B2 lands the registered, Expert-gated root screen as a scaffold: the global
//! nav header, the left nav rail, and an island content panel. The empty state
//! and the card grid (FR-2 / FR-3) are built on top in B3; the page-scoped
//! masternode pill is wired in B7.

use std::sync::Arc;

use eframe::egui::{self, RichText};

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_wallet_only_spec};
use crate::ui::theme::DashColors;
use crate::ui::{RootScreenType, ScreenLike};

/// Root screen for the Masternodes section.
pub struct MasternodesScreen {
    pub app_context: Arc<AppContext>,
}

impl MasternodesScreen {
    /// Construct the Masternodes screen. Follows the project convention:
    /// constructors handle errors internally and return `Self`; the scaffold has
    /// nothing to fail on yet.
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
        }
    }
}

impl ScreenLike for MasternodesScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        // TODO: replace the wallet-only spec with the page-scoped masternode
        // pill (IdentityPillScope::PageScopedObject) in B7.
        let mut action = add_top_panel_with_global_nav(
            ui,
            &self.app_context,
            subdued_wallet_only_spec("Masternodes", RootScreenType::RootScreenMasternodes),
            vec![],
        );

        action |= add_left_panel(ui, &self.app_context, RootScreenType::RootScreenMasternodes);

        action |= island_central_panel(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let dark_mode = ui.style().visuals.dark_mode;
            // Placeholder content; B3 renders the empty state + card grid here.
            ui.label(RichText::new("Masternodes").color(DashColors::text_primary(dark_mode)));
            AppAction::None
        });

        action
    }
}
