use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::platform::withdrawals::{WithdrawStatusData, WithdrawalsTask};
use crate::platform::{BackendTask, BackendTaskSuccessResult};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::Utc;
use dash_sdk::dpp::dash_to_credits;
use dash_sdk::dpp::document::DocumentV0Getters;
use egui::{Context, Ui};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;
use std::sync::{Arc, Mutex};

pub struct WithdrawsStatusScreen {
    pub app_context: Arc<AppContext>,
    data: Arc<Mutex<Option<WithdrawStatusData>>>,
    error_message: Option<String>,
}

impl WithdrawsStatusScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            data: Arc::new(Mutex::new(None)),
            error_message: None,
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {}

    fn show_output(&mut self, ui: &mut egui::Ui) {
        if self.error_message.is_some() {
            ui.centered_and_justified(|ui| {
                ui.heading(self.error_message.as_ref().unwrap());
            });
        } else {
            let lock_data = self.data.lock().unwrap_or_else(|poisoned| {
                // Mutex is poisoned, trying to recover the inner data
                poisoned.into_inner()
            });

            if let Some(ref data) = *lock_data {
                self.show_withdraws_data(ui, data);
            }
        }
    }

    fn show_withdraws_data(&self, ui: &mut egui::Ui, data: &WithdrawStatusData) {
        ui.heading("General Information");
        ui.separator();
        ui.label(format!(
            "Total withdrawals amount: {} DASH",
            data.total_amount as f64 / (dash_to_credits!(1) as f64)
        ));
        ui.label(format!(
            "Recent withdrawals amount: {} DASH",
            data.recent_withdrawal_amounts as f64 / (dash_to_credits!(1) as f64)
        ));
        ui.label(format!(
            "Daily withdrawals limit: {} DASH",
            data.daily_withdrawal_limit as f64 / (dash_to_credits!(1) as f64)
        ));
        ui.label(format!(
            "Total credits on Platform: {} DASH",
            data.total_credits_on_platform as f64 / (dash_to_credits!(1) as f64)
        ));
        ui.add_space(30.0);
        ui.heading(format!("Withdrawals ({})", data.withdrawals.len()));
        ui.separator();
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::initial(150.0).resizable(true)) // Date / Time
            .column(Column::initial(80.0).resizable(true)) // Status
            .column(Column::initial(140.0).resizable(true)) // Amount
            .column(Column::initial(350.0).resizable(true)) // Origin
            .column(Column::initial(320.0).resizable(true)) // Destination
            .header(20.0, |mut header| {
                header.col(|ui| {
                    ui.heading("Date / Time");
                });
                header.col(|ui| {
                    ui.heading("Status");
                });
                header.col(|ui| {
                    ui.heading("Amount");
                });
                header.col(|ui| {
                    ui.heading("Owner ID");
                });
                header.col(|ui| {
                    ui.heading("Destination");
                });
            })
            .body(|mut body| {
                for record in &data.withdrawals {
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&record.date_time.format("%Y-%m-%d %H:%M:%S").to_string());
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", &record.status));
                        });
                        row.col(|ui| {
                            ui.label(format!(
                                "{} DASH",
                                record.amount as f64 / (dash_to_credits!(1) as f64)
                            ));
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", &record.owner_id));
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", &record.address));
                        });
                    });
                }
            });
    }
}

impl ScreenLike for WithdrawsStatusScreen {
    fn refresh(&mut self) {
        let mut lock_data = self.data.lock().unwrap_or_else(|poisoned| {
            // Mutex is poisoned, trying to recover the inner data
            poisoned.into_inner()
        });
        *lock_data = None;
        self.error_message = None;
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some(message.to_string());
    }
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::WithdrawalStatus(data) = backend_task_success_result {
            let mut lock_data = self.data.lock().unwrap_or_else(|poisoned| {
                // Mutex is poisoned, trying to recover the inner data
                poisoned.into_inner()
            });
            *lock_data = Some(data);
            self.error_message = None;
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let query = (
            "Refresh",
            DesiredAppAction::BackendTask(BackendTask::WithdrawalTask(
                WithdrawalsTask::QueryWithdrawals,
            )),
        );
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            vec![query],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWithdrawsStatus,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_input_field(ui);
            self.show_output(ui);
        });

        action
    }
}