use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::ui::components::contract_chooser_panel::add_contract_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use egui::Context;
use std::sync::Arc;

pub struct DocumentQueryScreen {
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    contract_search_term: String,
}

impl DocumentQueryScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            error_message: None,
            contract_search_term: String::new(),
        }
    }

    fn dismiss_error(&mut self) {
        self.error_message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.error_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            // Automatically dismiss the error message after 5 seconds
            if elapsed.num_seconds() > 5 {
                self.dismiss_error();
            }
        }
    }
}

impl ScreenLike for DocumentQueryScreen {
    fn refresh(&mut self) {}

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();
        let add_contract_button = (
            "Add Contracts",
            DesiredAppAction::AddScreenType(ScreenType::AddContracts),
        );
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Document Queries", AppAction::None)],
            vec![add_contract_button],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        action |=
            add_contract_chooser_panel(ctx, &mut self.contract_search_term, &self.app_context);

        action
    }
}
