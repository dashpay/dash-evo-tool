use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::MessageBanner;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::ComponentStyles;
use crate::ui::{MessageType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery};
use std::sync::Arc;

use super::tokens_screen::IdentityTokenBasicInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum FetchStatus {
    NotFetching,
    Fetching(DateTime<Utc>),
}

impl FetchStatus {
    fn stop_if_fetch_in_flight(
        &mut self,
        expected: Option<&BackendTaskContext>,
        completed: &BackendTaskContext,
    ) -> bool {
        if matches!(self, Self::Fetching(_)) && expected == Some(completed) {
            *self = Self::NotFetching;
            true
        } else {
            false
        }
    }
}

pub struct ViewTokenClaimsScreen {
    pub identity_token_basic_info: IdentityTokenBasicInfo,
    pub new_claims_query: DocumentQuery,
    fetch_status: FetchStatus,
    pub app_context: Arc<AppContext>,
    claims: Vec<Document>,
    pending_fetch_context: Option<BackendTaskContext>,
}

impl ViewTokenClaimsScreen {
    pub fn new(
        identity_token_basic_info: IdentityTokenBasicInfo,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            identity_token_basic_info: identity_token_basic_info.clone(),
            new_claims_query: DocumentQuery {
                select: SelectProjection::documents(),
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
                group_by: Vec::new(),
                having: Vec::new(),
                order_by_clauses: vec![],
                limit: 0,
                offset: None,
                start: None,
            },
            fetch_status: FetchStatus::NotFetching,
            app_context: app_context.clone(),
            claims: vec![],
            pending_fetch_context: None,
        }
    }

    fn correlate_fetch_action(&mut self, action: AppAction) -> AppAction {
        let AppAction::BackendTask(task) = action else {
            return action;
        };
        if !matches!(
            BackendTaskContext::from(&task),
            BackendTaskContext::FetchDocuments(_)
        ) {
            return AppAction::BackendTask(task);
        }

        let context = BackendTaskContext::for_dispatch(&task);
        self.pending_fetch_context = Some(context.clone());
        self.fetch_status = FetchStatus::Fetching(Utc::now());
        AppAction::BackendTaskWithContext { task, context }
    }
}

impl ScreenLike for ViewTokenClaimsScreen {
    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if self
            .fetch_status
            .stop_if_fetch_in_flight(self.pending_fetch_context.as_ref(), context)
        {
            self.pending_fetch_context = None;
        }
    }

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.dispatched_document_fetch() && self.pending_fetch_context.as_ref() != Some(context)
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        backend_task_success_result: BackendTaskSuccessResult,
    ) {
        let BackendTaskSuccessResult::Documents(documents) = backend_task_success_result else {
            return;
        };
        if !context.is_fetch_documents()
            || !self
                .fetch_status
                .stop_if_fetch_in_flight(self.pending_fetch_context.as_ref(), context)
        {
            return;
        }

        self.pending_fetch_context = None;
        self.claims = documents.into_iter().filter_map(|(_, doc)| doc).collect();

        if self.claims.is_empty() {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "No claims found",
                MessageType::Info,
            );
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        // Top panel
        let mut action = add_top_panel(
            ui,
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
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ui, &self.app_context);

        // Central panel
        island_central_panel(ui, |ui| {
            ui.heading("View Token Claims");
            ui.add_space(10.0);

            if ComponentStyles::add_primary_button(ui, "Fetch claims").clicked() {
                action |= AppAction::BackendTask(BackendTask::DocumentTask(Box::new(
                    DocumentTask::FetchDocuments(self.new_claims_query.clone()),
                )));
            }

            // Message display is handled by the global MessageBanner

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

        self.correlate_fetch_action(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTaskContext;
    use dash_sdk::dpp::data_contracts::SystemDataContract;
    use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    fn query(limit: u32) -> DocumentQuery {
        let contract =
            load_system_data_contract(SystemDataContract::TokenHistory, PlatformVersion::latest())
                .expect("token history contract");
        let mut query = DocumentQuery::new(Arc::new(contract), "claim").expect("claim query");
        query.limit = limit;
        query
    }

    #[test]
    fn in_flight_claim_fetch_failure_requires_the_matching_backend_task() {
        let expected = BackendTaskContext::FetchDocuments(Box::new(query(10)));
        let different_query = BackendTaskContext::FetchDocuments(Box::new(query(20)));
        let mut status = FetchStatus::Fetching(Utc::now());
        assert!(!status.stop_if_fetch_in_flight(Some(&expected), &BackendTaskContext::Other));
        assert!(!status.stop_if_fetch_in_flight(Some(&expected), &BackendTaskContext::Unknown));
        assert!(!status.stop_if_fetch_in_flight(Some(&expected), &different_query));
        assert!(!status.stop_if_fetch_in_flight(
            Some(&expected),
            &BackendTaskContext::FetchDocumentsPage(Box::new(query(10))),
        ));
        assert!(matches!(status, FetchStatus::Fetching(_)));

        assert!(status.stop_if_fetch_in_flight(Some(&expected), &expected));
        assert_eq!(status, FetchStatus::NotFetching);

        assert!(!status.stop_if_fetch_in_flight(Some(&expected), &expected));
        assert_eq!(status, FetchStatus::NotFetching);
    }

    #[test]
    fn claim_success_requires_the_matching_in_flight_context() {
        let tmp = tempfile::tempdir().expect("temp data dir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = ViewTokenClaimsScreen::new(
            IdentityTokenBasicInfo {
                token_id: Identifier::from([1; 32]),
                token_alias: "Test token".to_string(),
                identity_id: Identifier::from([2; 32]),
                contract_id: Identifier::from([3; 32]),
                token_position: 0,
            },
            &app_context,
        );
        let expected =
            BackendTaskContext::FetchDocuments(Box::new(screen.new_claims_query.clone()));
        let mut different_query = screen.new_claims_query.clone();
        different_query.limit = 20;
        let different_context = BackendTaskContext::FetchDocuments(Box::new(different_query));

        screen.fetch_status = FetchStatus::Fetching(Utc::now());
        screen.pending_fetch_context = Some(expected.clone());
        screen.display_backend_task_result(
            &different_context,
            BackendTaskSuccessResult::Documents(Default::default()),
        );

        assert!(matches!(screen.fetch_status, FetchStatus::Fetching(_)));
        assert_eq!(screen.pending_fetch_context.as_ref(), Some(&expected));

        screen.display_backend_task_result(
            &expected,
            BackendTaskSuccessResult::Documents(Default::default()),
        );

        assert_eq!(screen.fetch_status, FetchStatus::NotFetching);
        assert_eq!(screen.pending_fetch_context, None);

        screen.display_backend_task_result(
            &expected,
            BackendTaskSuccessResult::Documents(Default::default()),
        );

        assert_eq!(screen.fetch_status, FetchStatus::NotFetching);
        assert_eq!(screen.pending_fetch_context, None);
    }

    #[test]
    fn identical_claim_redispatch_ignores_the_first_error_and_accepts_the_second_success() {
        let tmp = tempfile::tempdir().expect("temp data dir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = ViewTokenClaimsScreen::new(
            IdentityTokenBasicInfo {
                token_id: Identifier::from([1; 32]),
                token_alias: "Test token".to_string(),
                identity_id: Identifier::from([2; 32]),
                contract_id: Identifier::from([3; 32]),
                token_position: 0,
            },
            &app_context,
        );
        let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocuments(
            screen.new_claims_query.clone(),
        )));

        let first_action = screen.correlate_fetch_action(AppAction::BackendTask(task.clone()));
        let AppAction::BackendTaskWithContext {
            context: first_context,
            ..
        } = first_action
        else {
            panic!("claim fetch must carry its dispatch context");
        };
        let second_action = screen.correlate_fetch_action(AppAction::BackendTask(task));
        let AppAction::BackendTaskWithContext {
            context: second_context,
            ..
        } = second_action
        else {
            panic!("claim fetch must carry its dispatch context");
        };

        assert_ne!(first_context, second_context);
        assert!(
            screen
                .should_suppress_backend_task_error(&first_context, &TaskError::NoIdentitiesFound)
        );
        screen.display_backend_task_error(&first_context, &TaskError::NoIdentitiesFound);
        assert!(matches!(screen.fetch_status, FetchStatus::Fetching(_)));
        assert_eq!(screen.pending_fetch_context.as_ref(), Some(&second_context));

        screen.display_backend_task_result(
            &second_context,
            BackendTaskSuccessResult::Documents(Default::default()),
        );

        assert_eq!(screen.fetch_status, FetchStatus::NotFetching);
        assert_eq!(screen.pending_fetch_context, None);
    }
}
