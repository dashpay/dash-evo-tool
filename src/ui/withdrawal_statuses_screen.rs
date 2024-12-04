use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::withdrawal_statuses::{
    WithdrawRecord, WithdrawStatusData, WithdrawalsTask,
};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::dash_to_credits;
use dash_sdk::dpp::data_contracts::withdrawals_contract::WithdrawalStatus;
use egui::{Color32, ComboBox, Context, Stroke, Ui, Vec2};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, RwLock};

pub struct WithdrawsStatusScreen {
    pub app_context: Arc<AppContext>,
    requested_data: bool,
    first_load: bool,
    data: Arc<RwLock<Option<WithdrawStatusData>>>,
    sort_column: Option<SortColumn>,
    sort_ascending: bool,
    filter_status_queued: bool,
    filter_status_pooled: bool,
    filter_status_broadcasted: bool,
    filter_status_complete: bool,
    filter_status_expired: bool,
    filter_status_mix: Vec<WithdrawalStatus>,
    pagination_current_page: usize,
    pagination_items_per_page: PaginationItemsPerPage,
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

#[derive(Clone, Copy, PartialEq)]
enum PaginationItemsPerPage {
    Items10 = 10,
    Items15 = 15,
    Items20 = 20,
    Items30 = 30,
    Items50 = 50,
}

impl From<PaginationItemsPerPage> for u32 {
    fn from(item: PaginationItemsPerPage) -> Self {
        item as u32
    }
}

impl WithdrawsStatusScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            data: Arc::new(RwLock::new(None)),
            first_load: true,
            requested_data: false,
            sort_ascending: false,
            sort_column: Some(SortColumn::DateTime),
            error_message: None,
            filter_status_queued: true,
            filter_status_pooled: true,
            filter_status_broadcasted: true,
            filter_status_complete: true,
            filter_status_expired: false,
            filter_status_mix: vec![
                WithdrawalStatus::QUEUED,
                WithdrawalStatus::POOLED,
                WithdrawalStatus::BROADCASTED,
                WithdrawalStatus::COMPLETE,
                WithdrawalStatus::EXPIRED,
            ],
            pagination_current_page: 0,
            pagination_items_per_page: PaginationItemsPerPage::Items15,
        }
    }

    fn show_input_field(&mut self, _ui: &mut Ui) {}

    fn show_output(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut app_action = AppAction::None;
        if self.first_load {
            self.first_load = false;
            self.requested_data = true;
            app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                WithdrawalsTask::QueryWithdrawals(
                    self.filter_status_mix.clone(),
                    self.pagination_items_per_page.into(),
                    None,
                    true,
                    true,
                ),
            ));
        }
        if self.requested_data {
            ui.centered_and_justified(|ui| {
                self.show_spinner(ui, 75.0);
            });
        }
        if self.error_message.is_some() {
            ui.centered_and_justified(|ui| {
                ui.heading(self.error_message.as_ref().unwrap());
            });
        } else {
            let lock_data = self.data.read().unwrap().clone();

            if let Some(mut data) = lock_data {
                let sorted_data = self.sort_withdraws_data(data.withdrawals.as_slice());
                data.withdrawals = sorted_data;
                app_action |= self.show_withdraws_data(ui, &data);
            } else {
                self.requested_data = true;
                app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                    WithdrawalsTask::QueryWithdrawals(
                        self.filter_status_mix.clone(),
                        self.pagination_items_per_page.into(),
                        None,
                        true,
                        true,
                    ),
                ));
            }
        }
        app_action
    }

    fn sort_withdraws_data(&self, data: &[WithdrawRecord]) -> Vec<WithdrawRecord> {
        let mut result_data = data.to_vec();
        if let Some(column) = self.sort_column {
            let compare = |a: &WithdrawRecord, b: &WithdrawRecord| -> std::cmp::Ordering {
                let ord = match column {
                    SortColumn::DateTime => a.date_time.cmp(&b.date_time),
                    SortColumn::Status => (a.status as u8).cmp(&(b.status as u8)),
                    SortColumn::Amount => a.amount.cmp(&b.amount),
                    SortColumn::OwnerId => a.owner_id.cmp(&b.owner_id),
                    SortColumn::Destination => a.address.cmp(&b.address),
                };
                if self.sort_ascending {
                    ord
                } else {
                    ord.reverse()
                }
            };
            result_data.sort_by(compare);
        }
        result_data
    }

    fn handle_column_click(&mut self, current_sort: SortColumn) {
        if self.sort_column == Some(current_sort) {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_column = Some(current_sort);
            self.sort_ascending = true;
        }
    }

    fn show_withdraws_data(&mut self, ui: &mut egui::Ui, data: &WithdrawStatusData) -> AppAction {
        let mut app_action = AppAction::None;
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

                if ui
                    .checkbox(&mut self.filter_status_queued, "Queued")
                    .changed()
                {
                    self.util_build_combined_filter_status_mix();
                    self.requested_data = true;
                    let mut lock_data = self.data.write().unwrap();
                    *lock_data = None;
                    app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                        WithdrawalsTask::QueryWithdrawals(
                            self.filter_status_mix.clone(),
                            self.pagination_items_per_page.into(),
                            None,
                            true,
                            true,
                        ),
                    ));
                }
                ui.add_space(8.0);
                if ui
                    .checkbox(&mut self.filter_status_pooled, "Pooled")
                    .changed()
                {
                    self.util_build_combined_filter_status_mix();
                    self.requested_data = true;
                    let mut lock_data = self.data.write().unwrap();
                    *lock_data = None;
                    app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                        WithdrawalsTask::QueryWithdrawals(
                            self.filter_status_mix.clone(),
                            self.pagination_items_per_page.into(),
                            None,
                            true,
                            true,
                        ),
                    ));
                }
                ui.add_space(8.0);
                if ui
                    .checkbox(&mut self.filter_status_broadcasted, "Broadcasted")
                    .changed()
                {
                    self.util_build_combined_filter_status_mix();
                    self.requested_data = true;
                    let mut lock_data = self.data.write().unwrap();
                    *lock_data = None;
                    app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                        WithdrawalsTask::QueryWithdrawals(
                            self.filter_status_mix.clone(),
                            self.pagination_items_per_page.into(),
                            None,
                            true,
                            true,
                        ),
                    ));
                }
                ui.add_space(8.0);
                if ui
                    .checkbox(&mut self.filter_status_complete, "Complete")
                    .changed()
                {
                    self.util_build_combined_filter_status_mix();
                    self.requested_data = true;
                    let mut lock_data = self.data.write().unwrap();
                    *lock_data = None;
                    app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                        WithdrawalsTask::QueryWithdrawals(
                            self.filter_status_mix.clone(),
                            self.pagination_items_per_page.into(),
                            None,
                            true,
                            true,
                        ),
                    ));
                }
                ui.add_space(8.0);
                if ui
                    .checkbox(&mut self.filter_status_expired, "Expired")
                    .changed()
                {
                    self.util_build_combined_filter_status_mix();
                    self.requested_data = true;
                    let mut lock_data = self.data.write().unwrap();
                    *lock_data = None;
                    app_action |= AppAction::BackendTask(BackendTask::WithdrawalTask(
                        WithdrawalsTask::QueryWithdrawals(
                            self.filter_status_mix.clone(),
                            self.pagination_items_per_page.into(),
                            None,
                            true,
                            true,
                        ),
                    ));
                }
                ui.add_space(8.0);
            });
        });

        ui.add_space(30.0);
        ui.heading(format!("Withdrawals ({})", data.withdrawals.len()));
        let mut selected = self.pagination_items_per_page;
        let old_selected = selected;
        ComboBox::from_label("Items per page")
            .selected_text(format!("{}", selected as usize))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, PaginationItemsPerPage::Items10, "10");
                ui.selectable_value(&mut selected, PaginationItemsPerPage::Items15, "15");
                ui.selectable_value(&mut selected, PaginationItemsPerPage::Items20, "20");
                ui.selectable_value(&mut selected, PaginationItemsPerPage::Items30, "30");
                ui.selectable_value(&mut selected, PaginationItemsPerPage::Items50, "50");
            });
        if selected != old_selected {
            self.pagination_items_per_page = selected;
        }
        let total_pages = (data.withdrawals.len() + (self.pagination_items_per_page as usize) - 1)
            / (self.pagination_items_per_page as usize);
        if total_pages > 0 {
            let current_page = self
                .pagination_current_page
                .min(total_pages.saturating_sub(1)); // Clamp to valid page range
                                                     // Calculate the slice of data for the current page
            let start_index = current_page * (self.pagination_items_per_page as usize);
            let end_index = (start_index + (self.pagination_items_per_page as usize))
                .min(data.withdrawals.len());
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
                            self.handle_column_click(SortColumn::DateTime);
                        }
                    });
                    header.col(|ui| {
                        if ui.selectable_label(false, "Status").clicked() {
                            self.handle_column_click(SortColumn::Status);
                        }
                    });
                    header.col(|ui| {
                        if ui.selectable_label(false, "Amount").clicked() {
                            self.handle_column_click(SortColumn::Amount);
                        }
                    });
                    header.col(|ui| {
                        if ui.selectable_label(false, "Owner ID").clicked() {
                            self.handle_column_click(SortColumn::OwnerId);
                        }
                    });
                    header.col(|ui| {
                        if ui.selectable_label(false, "Destination").clicked() {
                            self.handle_column_click(SortColumn::Destination);
                        }
                    });
                })
                .body(|mut body| {
                    for record in &data.withdrawals[start_index..end_index] {
                        if self.filter_status_mix.contains(&record.status) {
                            body.row(18.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(
                                        &record.date_time.format("%Y-%m-%d %H:%M:%S").to_string(),
                                    );
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
                    }
                });
            // Pagination controls at the bottom
            ui.horizontal(|ui| {
                if ui.button("Previous").clicked() && current_page > 0 {
                    self.pagination_current_page = current_page - 1
                }
                ui.label(format!("Page {}/{}", current_page + 1, total_pages));

                if ui.button("Next").clicked() && current_page < total_pages {
                    self.pagination_current_page = current_page + 1;
                    if current_page == total_pages - 1 {
                        app_action = AppAction::BackendTask(BackendTask::WithdrawalTask(
                            WithdrawalsTask::QueryWithdrawals(
                                self.filter_status_mix.clone(),
                                self.pagination_items_per_page.into(),
                                data.withdrawals
                                    .last()
                                    .map(|withdrawal_record| withdrawal_record.document_id),
                                false,
                                false,
                            ),
                        ));
                    }
                }
            });
        } else {
            ui.centered_and_justified(|ui| {
                ui.heading("No withdrawals");
            });
        }
        app_action
    }

    fn show_spinner(&self, ui: &mut egui::Ui, size: f32) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }

        let painter = ui.painter_at(rect);
        let center = rect.center();
        let time = ui.input(|i| i.time); // Time in seconds since the program started

        // Spinner parameters
        let segments = 12;
        let radius = size * 0.5;
        let thickness = size * 0.1;
        let rotation_speed = std::f32::consts::TAU / 1.5; // One full rotation every 1.5 seconds

        for i in 0..segments {
            let t = i as f32 / segments as f32;
            let angle = t * std::f32::consts::TAU - ((time as f32) * rotation_speed);
            let alpha = t;

            let color = Color32::from_rgba_premultiplied(150, 150, 150, (alpha * 255.0) as u8);

            let start = center + Vec2::angled(angle) * (radius - thickness);
            let end = center + Vec2::angled(angle) * radius;

            painter.line_segment([start, end], Stroke::new(thickness * (1.0 - t), color));
        }
    }
    fn util_build_combined_filter_status_mix(&mut self) {
        let mut res = vec![];
        if self.filter_status_queued {
            res.push(WithdrawalStatus::QUEUED);
        }
        if self.filter_status_pooled {
            res.push(WithdrawalStatus::POOLED);
        }
        if self.filter_status_broadcasted {
            res.push(WithdrawalStatus::BROADCASTED);
        }
        if self.filter_status_complete {
            res.push(WithdrawalStatus::COMPLETE);
        }
        if self.filter_status_expired {
            res.push(WithdrawalStatus::EXPIRED);
        }
        self.filter_status_mix = res;
    }
}

impl ScreenLike for WithdrawsStatusScreen {
    fn refresh(&mut self) {
        let mut lock_data = self.data.write().unwrap();
        *lock_data = None;
        self.error_message = None;
    }

    fn display_message(&mut self, message: &str, _message_type: MessageType) {
        self.error_message = Some(message.to_string());
        self.requested_data = false;
    }
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::WithdrawalStatus(data) = backend_task_success_result {
            let mut lock_data = self.data.write().unwrap();
            if let Some(old_data) = lock_data.as_mut() {
                old_data.merge_with_data(data)
            } else {
                *lock_data = Some(data.try_into().expect("expected data to already exist"));
            }
            self.error_message = None;
            self.requested_data = false;
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let query = (
            "Refresh",
            DesiredAppAction::BackendTask(BackendTask::WithdrawalTask(
                WithdrawalsTask::QueryWithdrawals(
                    self.filter_status_mix.clone(),
                    self.pagination_items_per_page.into(),
                    None,
                    true,
                    true,
                ),
            )),
        );
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Withdrawal Statuses", AppAction::None)],
            vec![query],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWithdrawsStatus,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_input_field(ui);
            action |= self.show_output(ui);
        });

        action
    }
}
