use crate::app::AppAction;
use crate::backend_task::platform_info::{PlatformInfoTaskRequestType, PlatformInfoTaskResult};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::version::PlatformVersion;
use eframe::egui::{self, ScrollArea, Ui};
use std::sync::Arc;

pub struct PlatformInfoScreen {
    pub(crate) app_context: Arc<AppContext>,
    platform_version: Option<&'static PlatformVersion>,
    core_chain_lock_height: Option<u32>,
    network: Network,
    current_result: Option<String>,
    current_result_title: Option<String>,
    active_tasks: std::collections::HashSet<String>,
}

impl PlatformInfoScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            platform_version: None,
            core_chain_lock_height: None,
            network: app_context.network,
            current_result: None,
            current_result_title: None,
            active_tasks: std::collections::HashSet::new(),
        }
    }

    /// Drop the fetches and the result that belong to the previous network.
    pub(crate) fn reset_for_network_switch(&mut self) {
        self.platform_version = None;
        self.core_chain_lock_height = None;
        self.network = self.app_context.network;
        self.current_result = None;
        self.current_result_title = None;
        self.active_tasks.clear();
    }

    fn trigger_task(
        &mut self,
        task_type: PlatformInfoTaskRequestType,
        task_name: &str,
    ) -> AppAction {
        if !self.active_tasks.contains(task_name) {
            self.active_tasks.insert(task_name.to_string());
            let task = BackendTask::PlatformInfo(task_type);
            return AppAction::BackendTask(task);
        }
        AppAction::None
    }

    fn render_action_buttons(&mut self, ui: &mut Ui) -> AppAction {
        let button_tasks = vec![
            (
                "basic_info",
                "Fetch Basic Platform Info",
                PlatformInfoTaskRequestType::BasicPlatformInfo,
            ),
            (
                "epoch_info",
                "Fetch Current Epoch Info",
                PlatformInfoTaskRequestType::CurrentEpochInfo,
            ),
            (
                "total_credits",
                "Fetch Total Credits on Platform",
                PlatformInfoTaskRequestType::TotalCreditsOnPlatform,
            ),
            (
                "version_voting",
                "Fetch Version Voting State",
                PlatformInfoTaskRequestType::CurrentVersionVotingState,
            ),
            (
                "validator_set",
                "Fetch Validator Set Info",
                PlatformInfoTaskRequestType::CurrentValidatorSetInfo,
            ),
            (
                "withdrawals_queue",
                "Fetch Current Withdrawals in Queue",
                PlatformInfoTaskRequestType::CurrentWithdrawalsInQueue,
            ),
            (
                "recent_withdrawals",
                "Fetch Recent Withdrawal History",
                PlatformInfoTaskRequestType::RecentlyCompletedWithdrawals,
            ),
            (
                "shielded_pool",
                "Fetch Shielded Pool State",
                PlatformInfoTaskRequestType::ShieldedPoolState,
            ),
        ];

        let mut action = AppAction::None;

        ui.vertical(|ui| {
            for (task_id, button_text, task_type) in button_tasks {
                let is_loading = self.active_tasks.contains(task_id);

                let button = ui.add_enabled(!is_loading, egui::Button::new(button_text));
                if button.clicked() {
                    action = self.trigger_task(task_type, task_id);
                }
                ui.add_space(5.0);
            }
        });

        action
    }

    fn render_results(&mut self, ui: &mut Ui) {
        // Check if any task is loading
        if !self.active_tasks.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);

                // Show spinner with Dash blue color
                ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));

                ui.add_space(10.0);
                ui.heading("Loading...");
                ui.label("Fetching platform information from the network");
            });
            return;
        }

        // Display normal results
        if let Some(result) = &self.current_result {
            if let Some(title) = &self.current_result_title {
                ui.heading(title);
                ui.separator();
            }
            ui.label(result);
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.heading("No results yet");
                ui.label("Select an action from the left panel to fetch platform information.");
            });
        }
    }
}

impl ScreenLike for PlatformInfoScreen {
    fn refresh(&mut self) {
        // Unrelated background work sends refresh notifications, several a
        // second at times. Nothing here is read from local data, so a fetch in
        // flight and the result on screen both outlive them.
    }

    fn refresh_on_arrival(&mut self) {
        // Don't auto-refresh - let user trigger actions manually
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![("Tools", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenToolsPlatformInfoScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ui, self.app_context.as_ref());

        let panel_action = island_central_panel(ui, |ui| {
            ui.heading("Platform Information Tool");
            ui.separator();

            let mut button_action = AppAction::None;

            let available_height = ui.available_height();

            ui.horizontal(|ui| {
                // Left column: Action buttons only (fixed width)
                ui.allocate_ui_with_layout(
                    egui::vec2(280.0, available_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ScrollArea::vertical()
                            .id_salt("platform_buttons_scroll")
                            .max_height(available_height)
                            .show(ui, |ui| {
                                button_action = self.render_action_buttons(ui);
                            });
                    },
                );

                ui.separator();

                // Right column: Results (takes remaining space)
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), available_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ScrollArea::vertical()
                            .id_salt("platform_results_scroll")
                            .max_height(available_height)
                            .show(ui, |ui| {
                                self.render_results(ui);
                            });
                    },
                );
            });

            button_action
        });

        action |= panel_action;

        action
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.active_tasks.clear();
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::PlatformInfo(result) = backend_task_success_result {
            match result {
                PlatformInfoTaskResult::BasicPlatformInfo {
                    platform_version,
                    core_chain_lock_height,
                    network,
                } => {
                    self.platform_version = Some(platform_version);
                    self.core_chain_lock_height = core_chain_lock_height;
                    self.network = network;

                    // Format basic platform info for display
                    let basic_info = format!(
                        "Platform Version Information:\n\n\
                         • Protocol Version: {}\n\
                         • Fee Version: {:?}\n\
                         • Drive ABCI Version: {:?}\n\
                         • Drive Version: {:?}\n\
                         • DPP Version: {:?}\n\
                         • System Data Contracts:\n\
                           - DPNS: {}\n\
                           - Withdrawals: {}\n\n\
                         Network: {:?}\n\
                         Chain Lock Height: {}",
                        platform_version.protocol_version,
                        platform_version.fee_version,
                        platform_version.drive_abci,
                        platform_version.drive,
                        platform_version.dpp,
                        platform_version.system_data_contracts.dpns,
                        platform_version.system_data_contracts.withdrawals,
                        network,
                        core_chain_lock_height
                            .map_or("Not available".to_string(), |h| h.to_string())
                    );

                    self.current_result = Some(basic_info);
                    self.current_result_title = Some("Basic Platform Information".to_string());
                    self.active_tasks.remove("basic_info");
                    // Error state cleared on success
                }
                PlatformInfoTaskResult::TextResult(text) => {
                    // Find which task this result is for and set the title appropriately
                    let task_names = vec![
                        ("epoch_info", "Current Epoch Information"),
                        ("total_credits", "Total Credits on Platform"),
                        ("version_voting", "Protocol Version Voting State"),
                        ("validator_set", "Current Validator Set Information"),
                        ("withdrawals_queue", "Current Withdrawals in Queue"),
                        ("recent_withdrawals", "Recent Withdrawal History"),
                        ("shielded_pool", "Shielded Pool State"),
                    ];

                    // Try to identify which task completed based on active tasks
                    let mut title = "Platform Information".to_string();
                    for (task_id, task_display_name) in &task_names {
                        if self.active_tasks.contains(*task_id) {
                            title = task_display_name.to_string();
                            self.active_tasks.remove(*task_id);
                            break;
                        }
                    }

                    self.current_result = Some(text);
                    self.current_result_title = Some(title);
                    self.active_tasks.clear(); // Clear any remaining active tasks
                    // Error state cleared on success
                }
                PlatformInfoTaskResult::AddressBalance { .. } => {
                    // This result is handled by AddressBalanceScreen, not here
                }
                PlatformInfoTaskResult::Withdrawals { .. } => {
                    // Structured withdrawals are a programmatic (MCP/CLI) result;
                    // this screen uses the text variants instead.
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{test_app_context, test_app_context_for_network};
    use crate::ui::Screen;

    const EPOCH_RESULT: &str = "Current epoch: 42";

    fn epoch_result() -> BackendTaskSuccessResult {
        BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::TextResult(
            EPOCH_RESULT.to_owned(),
        ))
    }

    fn start_epoch_fetch(screen: &mut PlatformInfoScreen) {
        let action =
            screen.trigger_task(PlatformInfoTaskRequestType::CurrentEpochInfo, "epoch_info");
        assert!(
            matches!(action, AppAction::BackendTask(_)),
            "the premise: the fetch is dispatched"
        );
    }

    #[test]
    fn refresh_keeps_a_fetch_in_flight() {
        let dir = tempfile::tempdir().expect("context dir");
        let mut screen = PlatformInfoScreen::new(&test_app_context(dir.path()));
        start_epoch_fetch(&mut screen);

        screen.refresh();

        assert!(
            screen.active_tasks.contains("epoch_info"),
            "a refresh notification must not forget a fetch that is still running"
        );
    }

    #[test]
    fn refresh_keeps_the_result_on_screen() {
        let dir = tempfile::tempdir().expect("context dir");
        let mut screen = PlatformInfoScreen::new(&test_app_context(dir.path()));
        start_epoch_fetch(&mut screen);
        screen.refresh();
        screen.display_task_result(epoch_result());

        screen.refresh();

        assert_eq!(screen.current_result.as_deref(), Some(EPOCH_RESULT));
        assert_eq!(
            screen.current_result_title.as_deref(),
            Some("Current Epoch Information"),
            "the result keeps the title of the fetch that produced it"
        );
        assert!(screen.active_tasks.is_empty(), "the fetch has finished");
    }

    #[test]
    fn a_network_switch_drops_the_previous_network_state() {
        let old_dir = tempfile::tempdir().expect("old context dir");
        let new_dir = tempfile::tempdir().expect("new context dir");
        let mut screen = PlatformInfoScreen::new(&test_app_context(old_dir.path()));
        start_epoch_fetch(&mut screen);
        screen.display_task_result(epoch_result());
        start_epoch_fetch(&mut screen);

        let mut screen = Screen::PlatformInfoScreen(screen);
        screen.change_context(test_app_context_for_network(
            new_dir.path(),
            Network::Mainnet,
        ));

        let Screen::PlatformInfoScreen(screen) = screen else {
            panic!("the screen keeps its type across a network switch");
        };
        assert_eq!(screen.current_result, None);
        assert_eq!(screen.current_result_title, None);
        assert!(screen.active_tasks.is_empty());
        assert_eq!(screen.network, Network::Mainnet);
    }
}
