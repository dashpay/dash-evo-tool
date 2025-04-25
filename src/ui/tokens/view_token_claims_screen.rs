use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::{Document, DocumentQuery};
use egui::Color32;
use egui::Context;
use std::sync::Arc;

use super::tokens_screen::IdentityTokenBalance;

#[derive(Debug, Clone, PartialEq)]
pub enum FetchStatus {
    NotFetching,
    Fetching(DateTime<Utc>),
}

pub struct ViewTokenClaimsScreen {
    pub identity_token_balance: IdentityTokenBalance,
    pub new_claims_query: DocumentQuery,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    fetch_status: FetchStatus,
    pub app_context: Arc<AppContext>,
}

impl ViewTokenClaimsScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            identity_token_balance,
            new_claims_query: DocumentQuery {
                data_contract: app_context.token_history_contract.clone(),
                document_type_name: "claim".to_string(),
                where_clauses: vec![],
                order_by_clauses: vec![],
                limit: 0,
                start: None,
            },
            message: None,
            fetch_status: FetchStatus::NotFetching,
            app_context: app_context.clone(),
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
        match backend_task_success_result {
            BackendTaskSuccessResult::Documents(documents) => {
                self.fetch_status = FetchStatus::NotFetching;
                if !documents.is_empty() {
                    let claims: Vec<Document> =
                        documents.into_iter().filter_map(|(_, doc)| doc).collect();
                    let documents_string = claims
                        .iter()
                        .map(|doc| {
                            let amount_string = doc
                                .get("amount")
                                .unwrap_or(&Value::Text("None".to_string()))
                                .to_string();
                            let timestamp_string = doc.created_at().unwrap_or_default().to_string();
                            let block_height_string = doc
                                .created_at_block_height()
                                .unwrap_or_default()
                                .to_string();
                            let note_string = doc
                                .get("note")
                                .unwrap_or(&Value::Text("None".to_string()))
                                .to_string();

                            format!(
                                "Claim: Amount: {}, Timestamp: {}, Block Height: {}, Note: {}",
                                amount_string, timestamp_string, block_height_string, note_string
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.display_message(&documents_string, MessageType::Info);
                } else {
                    self.display_message("No claims found", MessageType::Info);
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_balance.token_name,
                    AppAction::PopScreen,
                ),
                ("View Claims", AppAction::None),
            ],
            vec![(
                "Refresh",
                DesiredAppAction::BackendTask(BackendTask::DocumentTask(
                    DocumentTask::FetchDocuments(self.new_claims_query.clone()),
                )),
            )],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("View Token Claims");
            ui.add_space(10.0);

            if ui.button("Fetch Claims").clicked() {
                action |= AppAction::BackendTask(BackendTask::DocumentTask(
                    DocumentTask::FetchDocuments(self.new_claims_query.clone()),
                ));
                self.fetch_status = FetchStatus::Fetching(Utc::now())
            }

            if let Some((msg, msg_type, _)) = &self.message {
                ui.add_space(10.0);
                match msg_type {
                    MessageType::Success => {
                        ui.colored_label(Color32::DARK_GREEN, msg);
                    }
                    MessageType::Error => {
                        ui.colored_label(Color32::DARK_RED, msg);
                    }
                    MessageType::Info => {
                        ui.label(msg);
                    }
                };
            }

            if self.fetch_status != FetchStatus::NotFetching {
                ui.add_space(10.0);
                match &self.fetch_status {
                    FetchStatus::Fetching(start_time) => {
                        let elapsed = Utc::now().signed_duration_since(*start_time);
                        ui.label(format!("Fetching... ({} seconds)", elapsed.num_seconds()));
                    }
                    _ => {}
                }
            }
        });

        action
    }
}
