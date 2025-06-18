use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery};
use egui::Context;
use egui::{Color32, RichText};
use std::sync::Arc;

use super::tokens_screen::IdentityTokenBasicInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum FetchStatus {
    NotFetching,
    Fetching(DateTime<Utc>),
}

pub struct ViewTokenClaimsScreen {
    pub identity_token_basic_info: IdentityTokenBasicInfo,
    pub new_claims_query: DocumentQuery,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    fetch_status: FetchStatus,
    pub app_context: Arc<AppContext>,
    claims: Vec<Document>,
}

impl ViewTokenClaimsScreen {
    pub fn new(
        identity_token_basic_info: IdentityTokenBasicInfo,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            identity_token_basic_info: identity_token_basic_info.clone(),
            new_claims_query: DocumentQuery {
                data_contract: app_context.token_history_contract.clone(),
                document_type_name: "claim".to_string(),
                where_clauses: vec![
                    WhereClause {
                        field: "tokenId".to_string(),
                        operator: WhereOperator::Equal,
                        value: Value::Identifier(identity_token_basic_info.token_id.into()),
                    },
                    WhereClause {
                        field: "recipientId".to_string(),
                        operator: WhereOperator::Equal,
                        value: Value::Identifier(identity_token_basic_info.identity_id.into()),
                    },
                ],
                order_by_clauses: vec![],
                limit: 0,
                start: None,
            },
            message: None,
            fetch_status: FetchStatus::NotFetching,
            app_context: app_context.clone(),
            claims: vec![],
        }
    }
}

impl ScreenLike for ViewTokenClaimsScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.message = Some((message.to_string(), MessageType::Success, Utc::now()));
            }
            MessageType::Error => {
                self.message = Some((message.to_string(), MessageType::Error, Utc::now()));
                if message.contains("Error fetching documents") {
                    self.fetch_status = FetchStatus::NotFetching;
                }
            }
            MessageType::Info => {
                self.message = Some((message.to_string(), MessageType::Info, Utc::now()));
            }
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::Documents(documents) = backend_task_success_result {
            self.fetch_status = FetchStatus::NotFetching;
            self.claims = documents.into_iter().filter_map(|(_, doc)| doc).collect();

            if self.claims.is_empty() {
                self.display_message("No claims found", MessageType::Info);
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        // Top panel
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_basic_info.token_alias,
                    AppAction::PopScreen,
                ),
                ("View Claims", AppAction::None),
            ],
            vec![(
                "Refresh",
                DesiredAppAction::BackendTask(Box::new(BackendTask::DocumentTask(Box::new(
                    DocumentTask::FetchDocuments(self.new_claims_query.clone()),
                )))),
            )],
        );

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        // Central panel
        island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            ui.heading("View Token Claims");
            ui.add_space(10.0);

            let fetch_button =
                egui::Button::new(RichText::new("Fetch claims").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);

            if ui.add(fetch_button).clicked() {
                action |= AppAction::BackendTask(BackendTask::DocumentTask(Box::new(
                    DocumentTask::FetchDocuments(self.new_claims_query.clone()),
                )));
                self.fetch_status = FetchStatus::Fetching(Utc::now())
            }

            if let Some((msg, msg_type, _)) = &self.message {
                ui.add_space(10.0);
                match msg_type {
                    MessageType::Success => {
                        ui.colored_label(DashColors::success_color(dark_mode), msg);
                    }
                    MessageType::Error => {
                        ui.colored_label(DashColors::error_color(dark_mode), msg);
                    }
                    MessageType::Info => {
                        ui.label(msg);
                    }
                };
            }

            if self.fetch_status != FetchStatus::NotFetching {
                ui.add_space(10.0);
                if let FetchStatus::Fetching(start_time) = &self.fetch_status {
                    let elapsed = Utc::now().signed_duration_since(*start_time);
                    ui.label(format!("Fetching... ({} seconds)", elapsed.num_seconds()));
                }
            }

            ui.add_space(10.0);

            if !self.claims.is_empty() {
                egui::ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        egui::Grid::new("claims_table")
                            .striped(false)
                            .spacing([20.0, 8.0])
                            .show(ui, |ui| {
                                // Header
                                ui.label("Amount");
                                ui.label("Timestamp");
                                ui.label("Block Height");
                                ui.label("Note");
                                ui.end_row();

                                for claim in &self.claims {
                                    // Amount
                                    let amount = match claim.get("amount") {
                                        Some(Value::U64(amount)) => amount.to_string(),
                                        Some(Value::I64(amount)) => {
                                            if *amount >= 0 {
                                                (*amount as u64).to_string()
                                            } else {
                                                format!("{}", amount)
                                            }
                                        }
                                        Some(Value::Text(s)) => s.clone(),
                                        Some(other) => other.to_string(),
                                        None => "None".to_string(),
                                    };

                                    // Timestamp
                                    let timestamp = match claim.created_at() {
                                        Some(ts) => {
                                            let dt =
                                                chrono::DateTime::from_timestamp_millis(ts as i64)
                                                    .map(|d| d.naive_utc())
                                                    .unwrap_or_else(|| {
                                                        chrono::DateTime::from_timestamp(0, 0)
                                                            .unwrap()
                                                            .naive_utc()
                                                    });
                                            dt.format("%Y-%m-%d %H:%M:%S").to_string()
                                        }
                                        None => "Unknown".to_string(),
                                    };

                                    // Block Height
                                    let block_height = claim
                                        .created_at_block_height()
                                        .map(|h| h.to_string())
                                        .unwrap_or_else(|| "Unknown".to_string());

                                    // Note
                                    let note = match claim.get("note") {
                                        Some(Value::Text(note)) => note.clone(),
                                        Some(other) => other.to_string(),
                                        None => "".to_string(),
                                    };

                                    ui.label(amount);
                                    ui.label(timestamp);
                                    ui.label(block_height);
                                    ui.label(note);
                                    ui.end_row();
                                }
                            });
                    });
            }
        });

        action
    }
}
