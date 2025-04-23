use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::platform::DocumentQuery;
use egui::Context;
use std::sync::Arc;

use super::tokens_screen::IdentityTokenBalance;

pub struct ViewTokenClaimsScreen {
    pub identity_token_balance: IdentityTokenBalance,
    pub new_claims_query: DocumentQuery,
    token_contract: Option<QualifiedContract>,
    token_configuration: Option<TokenConfiguration>,
    distribution_type: Option<TokenDistributionType>,
    error_message: Option<String>,
    pub app_context: Arc<AppContext>,
    show_confirmation_popup: bool,
    wallet_password: String,
    show_password: bool,
}

impl ViewTokenClaimsScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let token_contract = app_context
            .db
            .get_contract_by_id(identity_token_balance.data_contract_id, app_context)
            .ok()
            .flatten();

        let token_configuration = token_contract
            .as_ref()
            .map(|contract| {
                contract
                    .contract
                    .expected_token_configuration(identity_token_balance.token_position)
                    .ok()
            })
            .flatten()
            .cloned();

        Self {
            identity_token_balance,
            new_claims_query: DocumentQuery {
                data_contract: app_context.token_history_contract.clone(),
                document_type_name: "claims".to_string(),
                where_clauses: vec![],
                order_by_clauses: vec![],
                limit: 0,
                start: None,
            },
            token_contract,
            token_configuration,
            distribution_type: None,
            error_message: None,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            wallet_password: String::new(),
            show_password: false,
        }
    }
}

impl ScreenLike for ViewTokenClaimsScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {}
            MessageType::Error => {}
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {}

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let action = add_top_panel(
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
        });

        action
    }
}
