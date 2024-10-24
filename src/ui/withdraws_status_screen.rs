use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::platform::withdrawals::{WithdrawRecord, WithdrawStatusData, WithdrawalsTask};
use crate::platform::{BackendTask, BackendTaskSuccessResult};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::dash_to_credits;
use dash_sdk::dpp::data_contracts::withdrawals_contract::WithdrawalStatus;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use egui::{Context, Ui};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;
use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};

const ITEMS_PER_PAGE: usize = 20;

pub struct WithdrawsStatusScreen {
    pub app_context: Arc<AppContext>,
    data: Arc<Mutex<Option<WithdrawStatusData>>>,
    sort_column: Cell<Option<SortColumn>>,
    sort_ascending: Cell<bool>,
    filter_status_queued: Cell<bool>,
    filter_status_pooled: Cell<bool>,
    filter_status_broadcasted: Cell<bool>,
    filter_status_complete: Cell<bool>,
    filter_status_expired: Cell<bool>,
    filter_status_mix: RefCell<Vec<Value>>,
    pagination_current_page: Cell<usize>,
    error_message: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum SortColumn {
    DateTime,
    Status,
    Amount,
    OwnerId,
    Destination,
}

impl WithdrawsStatusScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            data: Arc::new(Mutex::new(None)),
            sort_ascending: Cell::from(true),
            sort_column: Cell::from(Some(SortColumn::DateTime)),
            error_message: None,
            filter_status_queued: Cell::new(true),
            filter_status_pooled: Cell::new(true),
            filter_status_broadcasted: Cell::new(true),
            filter_status_complete: Cell::new(true),
            filter_status_expired: Cell::new(false),
            filter_status_mix: RefCell::new(vec![
                Value::U8(WithdrawalStatus::QUEUED as u8),
                Value::U8(WithdrawalStatus::POOLED as u8),
                Value::U8(WithdrawalStatus::BROADCASTED as u8),
                Value::U8(WithdrawalStatus::COMPLETE as u8),
                Value::U8(WithdrawalStatus::EXPIRED as u8),
            ]),
            pagination_current_page: Cell::new(0),
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {}

    fn show_output(&mut self, ui: &mut egui::Ui) {
        if self.error_message.is_some() {
            ui.centered_and_justified(|ui| {
                ui.heading(self.error_message.as_ref().unwrap());
            });
        } else {
            let mut lock_data = self.data.lock().unwrap_or_else(|poisoned| {
                // Mutex is poisoned, trying to recover the inner data
                poisoned.into_inner()
            });

            if let Some(ref mut data) = *lock_data {
                self.sort_withdraws_data(&mut data.withdrawals);
                self.show_withdraws_data(ui, data);
            }
        }
    }

    fn sort_withdraws_data(&self, data: &mut Vec<WithdrawRecord>) {
        data.sort_by(|a, b| match self.sort_column.get() {
            Some(SortColumn::DateTime) => {
                if self.sort_ascending.get() {
                    a.date_time.cmp(&b.date_time)
                } else {
                    b.date_time.cmp(&a.date_time)
                }
            }
            Some(SortColumn::Status) => {
                if self.sort_ascending.get() {
                    (a.status as u8).cmp(&(b.status as u8))
                } else {
                    (b.status as u8).cmp(&(a.status as u8))
                }
            }
            Some(SortColumn::Amount) => {
                if self.sort_ascending.get() {
                    a.amount.cmp(&b.amount)
                } else {
                    b.amount.cmp(&a.amount)
                }
            }
            Some(SortColumn::OwnerId) => {
                if self.sort_ascending.get() {
                    a.owner_id.cmp(&b.owner_id)
                } else {
                    b.owner_id.cmp(&a.owner_id)
                }
            }
            Some(SortColumn::Destination) => {
                if self.sort_ascending.get() {
                    a.address.cmp(&b.address)
                } else {
                    b.address.cmp(&a.address)
                }
            }
            None => std::cmp::Ordering::Equal,
        });
    }

    fn show_withdraws_data(&self, ui: &mut egui::Ui, data: &WithdrawStatusData) {
        egui::Grid::new("general_info_grid")
            .num_columns(2)
            .spacing([20.0, 8.0]) // Adjust spacing as needed
            .show(ui, |ui| {
                ui.heading("General Information");
                ui.separator();
                ui.end_row();
                ui.label("Total withdrawals amount:");
                ui.label(format!(
                    "{:.2} DASH",
                    data.total_amount as f64 / (dash_to_credits!(1) as f64)
                ));
                ui.end_row();

                ui.label("Recent withdrawals amount:");
                ui.label(format!(
                    "{:.2} DASH",
                    data.recent_withdrawal_amounts as f64 / (dash_to_credits!(1) as f64)
                ));
                ui.end_row();

                ui.label("Daily withdrawals limit:");
                ui.label(format!(
                    "{:.2} DASH",
                    data.daily_withdrawal_limit as f64 / (dash_to_credits!(1) as f64)
                ));
                ui.end_row();

                ui.label("Total credits on Platform:");
                ui.label(format!(
                    "{:.2} DASH",
                    data.total_credits_on_platform as f64 / (dash_to_credits!(1) as f64)
                ));
                ui.end_row();
            });

        ui.add_space(30.0); // Optional spacing between the grids

        egui::Grid::new("filters_grid").show(ui, |ui| {
            ui.heading("Filters");
            ui.end_row();
            ui.horizontal(|ui| {
                ui.label("Filter by status:");
                ui.add_space(8.0); // Space after label
                let mut value = self.filter_status_queued.get();
                if ui.checkbox(&mut value, "Queued").changed() {
                    self.filter_status_queued.set(value);
                    self.util_build_combined_filter_status_mix();
                }
                ui.add_space(8.0);
                let mut value = self.filter_status_pooled.get();
                if ui.checkbox(&mut value, "Pooled").changed() {
                    self.filter_status_pooled.set(value);
                    self.util_build_combined_filter_status_mix();
                }
                ui.add_space(8.0);
                let mut value = self.filter_status_broadcasted.get();
                if ui.checkbox(&mut value, "Broadcasted").changed() {
                    self.filter_status_broadcasted.set(value);
                    self.util_build_combined_filter_status_mix();
                }
                ui.add_space(8.0);
                let mut value = self.filter_status_complete.get();
                if ui.checkbox(&mut value, "Complete").changed() {
                    self.filter_status_complete.set(value);
                    self.util_build_combined_filter_status_mix();
                }
                ui.add_space(8.0);
                let mut value = self.filter_status_expired.get();
                if ui.checkbox(&mut value, "Expired").changed() {
                    self.filter_status_expired.set(value);
                    self.util_build_combined_filter_status_mix();
                }
            });
        });
        ui.add_space(30.0);

        let total_pages = (data.withdrawals.len() + ITEMS_PER_PAGE - 1) / ITEMS_PER_PAGE;
        let mut current_page = self.pagination_current_page.get().min(total_pages - 1); // Clamp to valid page range

        // Calculate the slice of data for the current page
        let start_index = current_page * ITEMS_PER_PAGE;
        let end_index = (start_index + ITEMS_PER_PAGE).min(data.withdrawals.len());

        ui.heading(format!("Withdrawals ({})", data.withdrawals.len()));
        ui.separator();
        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::initial(150.0).resizable(true)) // Date / Time
            .column(Column::initial(80.0).resizable(true)) // Status
            .column(Column::initial(140.0).resizable(true)) // Amount
            .column(Column::initial(350.0).resizable(true)) // OwnerID
            .column(Column::initial(320.0).resizable(true)) // Destination
            .header(20.0, |mut header| {
                header.col(|ui| {
                    if ui.selectable_label(false, "Date / Time").clicked() {
                        if self.sort_column.get() == Some(SortColumn::DateTime) {
                            self.sort_ascending.set(!self.sort_ascending.get());
                        } else {
                            self.sort_column.set(Some(SortColumn::DateTime));
                            self.sort_ascending.set(true);
                        }
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(false, "Status").clicked() {
                        if self.sort_column.get() == Some(SortColumn::Status) {
                            self.sort_ascending.set(!self.sort_ascending.get());
                        } else {
                            self.sort_column.set(Some(SortColumn::Status));
                            self.sort_ascending.set(true);
                        }
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(false, "Amount").clicked() {
                        if self.sort_column.get() == Some(SortColumn::Amount) {
                            self.sort_ascending.set(!self.sort_ascending.get());
                        } else {
                            self.sort_column.set(Some(SortColumn::Amount));
                            self.sort_ascending.set(true);
                        }
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(false, "Owner ID").clicked() {
                        if self.sort_column.get() == Some(SortColumn::OwnerId) {
                            self.sort_ascending.set(!self.sort_ascending.get());
                        } else {
                            self.sort_column.set(Some(SortColumn::OwnerId));
                            self.sort_ascending.set(true);
                        }
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(false, "Destination").clicked() {
                        if self.sort_column.get() == Some(SortColumn::Destination) {
                            self.sort_ascending.set(!self.sort_ascending.get());
                        } else {
                            self.sort_column.set(Some(SortColumn::Destination));
                            self.sort_ascending.set(true);
                        }
                    }
                });
            })
            .body(|mut body| {
                for record in &data.withdrawals[start_index..end_index] {
                    body.row(18.0, |mut row| {
                        row.col(|ui| {
                            ui.label(&record.date_time.format("%Y-%m-%d %H:%M:%S").to_string());
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", &record.status));
                        });
                        row.col(|ui| {
                            ui.label(format!(
                                "{:.2} DASH",
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
        // Pagination controls at the bottom
        ui.horizontal(|ui| {
            if ui.button("Previous").clicked() && current_page > 0 {
                self.pagination_current_page.set(current_page - 1)
            }

            ui.label(format!("Page {}/{}", current_page + 1, total_pages));

            if ui.button("Next").clicked() && current_page < total_pages - 1 {
                self.pagination_current_page.set(current_page + 1)
            }
        });
    }

    fn util_build_combined_filter_status_mix(&self) {
        let mut res = vec![];
        if self.filter_status_queued.get() {
            res.push(Value::U8(WithdrawalStatus::QUEUED as u8));
        }
        if self.filter_status_pooled.get() {
            res.push(Value::U8(WithdrawalStatus::POOLED as u8));
        }
        if self.filter_status_broadcasted.get() {
            res.push(Value::U8(WithdrawalStatus::BROADCASTED as u8));
        }
        if self.filter_status_complete.get() {
            res.push(Value::U8(WithdrawalStatus::COMPLETE as u8));
        }
        if self.filter_status_expired.get() {
            res.push(Value::U8(WithdrawalStatus::EXPIRED as u8));
        }

        self.filter_status_mix.borrow_mut().clear();
        self.filter_status_mix.borrow_mut().extend(res);
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
                WithdrawalsTask::QueryWithdrawals(self.filter_status_mix.borrow().clone()),
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
