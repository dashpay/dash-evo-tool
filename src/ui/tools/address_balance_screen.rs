use crate::app::AppAction;
use crate::backend_task::platform_info::{PlatformInfoTaskRequestType, PlatformInfoTaskResult};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use eframe::egui::{self, Color32, Context, Frame, Margin, RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

pub struct AddressBalanceScreen {
    pub(crate) app_context: Arc<AppContext>,
    address_input: String,
    is_loading: bool,
    result: Option<AddressBalanceResult>,
    error_message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AddressBalanceResult {
    pub address: String,
    pub balance: u64,
    pub nonce: u32,
}

impl AddressBalanceScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            address_input: String::new(),
            is_loading: false,
            result: None,
            error_message: None,
        }
    }

    fn trigger_fetch(&mut self) -> AppAction {
        let address = self.address_input.trim().to_string();
        if address.is_empty() {
            self.error_message = Some("Please enter an address".to_string());
            return AppAction::None;
        }

        self.is_loading = true;
        self.error_message = None;
        self.result = None;

        let task =
            BackendTask::PlatformInfo(PlatformInfoTaskRequestType::FetchAddressBalance(address));
        AppAction::BackendTask(task)
    }

    fn render_input(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("Platform Address Balance Lookup");
        ui.add_space(10.0);

        ui.label("Enter a Platform address (dash1... or tdash1...):");
        ui.add_space(5.0);

        let text_edit = TextEdit::singleline(&mut self.address_input)
            .hint_text("dash1... or tdash1...")
            .desired_width(500.0);

        let response = ui.add(text_edit);

        // Submit on Enter key
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = self.trigger_fetch();
        }

        ui.add_space(10.0);

        let button = ui.add_enabled(
            !self.is_loading && !self.address_input.trim().is_empty(),
            egui::Button::new(if self.is_loading {
                "Loading..."
            } else {
                "Fetch Balance"
            }),
        );

        if button.clicked() {
            action = self.trigger_fetch();
        }

        action
    }

    fn render_result(&mut self, ui: &mut Ui) {
        if let Some(ref error) = self.error_message {
            ui.add_space(20.0);
            let error_color = Color32::from_rgb(255, 100, 100);
            let error = error.clone();
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Error: {}", error)).color(error_color));
                        ui.add_space(10.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.error_message = None;
                        }
                    });
                });
        }

        if let Some(ref result) = self.result {
            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("Result");
            ui.add_space(10.0);

            egui::Grid::new("address_balance_grid")
                .num_columns(2)
                .spacing([20.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Address:");
                    ui.monospace(&result.address);
                    ui.end_row();

                    ui.label("Balance:");
                    let credits = result.balance;
                    let dash = credits as f64 / 100_000_000_000.0; // credits to Dash
                    ui.monospace(format!("{} credits ({:.8} Dash)", credits, dash));
                    ui.end_row();

                    ui.label("Nonce:");
                    ui.monospace(format!("{}", result.nonce));
                    ui.end_row();
                });
        }
    }
}

impl ScreenLike for AddressBalanceScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Error {
            self.error_message = Some(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        self.is_loading = false;

        if let BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::AddressBalance {
            address,
            balance,
            nonce,
        }) = backend_task_success_result
        {
            self.result = Some(AddressBalanceResult {
                address,
                balance,
                nonce,
            });
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Tools", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenToolsAddressBalanceScreen,
        );
        action |= add_tools_subscreen_chooser_panel(ctx, &self.app_context);

        island_central_panel(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                action |= self.render_input(ui);
                self.render_result(ui);
            });
        });

        action
    }

    fn refresh(&mut self) {}

    fn refresh_on_arrival(&mut self) {}

    fn pop_on_success(&mut self) {}
}
