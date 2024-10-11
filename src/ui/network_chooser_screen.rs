use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::Network;
use eframe::egui::{self, Color32, Context, Ui};
use std::sync::Arc;

pub struct NetworkChooserScreen {
    pub app_context: Arc<AppContext>,
    current_network: Option<Network>,
    pub mainnet_core_status_online: bool,
    pub testnet_core_status_online: bool,
}

impl NetworkChooserScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let current_network = Some(app_context.network);
        Self {
            app_context: app_context.clone(),
            current_network,
            mainnet_core_status_online: false,
            testnet_core_status_online: false,
        }
    }

    /// Render the network selection table
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.heading("Choose Network");

        egui::Grid::new("network_grid")
            .striped(true)
            .show(ui, |ui| {
                // Header row
                ui.label("Network");
                ui.label("Status");
                ui.label("Select");
                ui.end_row();

                // Render Mainnet
                app_action |= self.render_network_row(ui, Network::Dash, "Mainnet");

                // Render Testnet
                app_action |= self.render_network_row(ui, Network::Testnet, "Testnet");
            });
        app_action
    }

    /// Render a single row for the network table
    fn render_network_row(&mut self, ui: &mut Ui, network: Network, name: &str) -> AppAction {
        let mut app_action = AppAction::None;
        ui.label(name);

        // Simulate checking network status
        let is_working = self.check_network_status(network);
        let status_color = if is_working {
            Color32::from_rgb(0, 255, 0) // Green if working
        } else {
            Color32::from_rgb(255, 0, 0) // Red if not working
        };

        // Display status indicator
        ui.colored_label(status_color, if is_working { "Online" } else { "Offline" });

        // Network selection
        let mut is_selected = self.current_network == Some(network);
        if ui.checkbox(&mut is_selected, "Select").clicked() {
            if is_selected {
                self.current_network = Some(network);
                app_action = AppAction::SwitchNetwork(network);
            }
        }

        ui.end_row();
        app_action
    }

    /// Simulate a function to check if the network is working
    fn check_network_status(&self, network: Network) -> bool {
        match network {
            Network::Dash => self.mainnet_core_status_online,
            Network::Testnet => self.testnet_core_status_online,
            _ => false,
        }
    }
}

impl ScreenLike for NetworkChooserScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            None,
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenNetworkChooser,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            action |= self.render_network_table(ui);
        });

        action
    }
}
