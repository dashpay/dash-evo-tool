use super::token_operation_base::{OperationStatus, TokenOperationBase};
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::{TokenResult, TokenTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::confirmation_dialog::ConfirmationDialog;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::render_group_action_text;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use eframe::egui::{self, Color32, Context};
use egui::RichText;
use std::sync::Arc;

pub struct ResumeTokensScreen {
    pub base: TokenOperationBase,
}

impl ResumeTokensScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        Self {
            base: TokenOperationBase::new(&identity_token_info, app_context, |c| {
                c.emergency_action_rules()
            }),
        }
    }

    fn build_resume_task(base: &mut TokenOperationBase) -> AppAction {
        let data_contract = Arc::new(base.identity_token_info.data_contract.contract.clone());

        let group_info = if base.group_action_id.is_some() {
            base.group.as_ref().map(|(pos, _)| {
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                    GroupStateTransitionInfo {
                        group_contract_position: *pos,
                        action_id: base.group_action_id.unwrap(),
                        action_is_proposer: false,
                    },
                )
            })
        } else {
            base.group.as_ref().map(|(pos, _)| {
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
            })
        };

        AppAction::BackendTask(BackendTask::TokenTask(Box::new(TokenTask::ResumeTokens {
            actor_identity: base.identity.clone(),
            data_contract,
            token_position: base.identity_token_info.token_position,
            signing_key: base.selected_key.clone().unwrap(),
            public_note: if base.group_action_id.is_some() {
                None
            } else {
                base.public_note.clone()
            },
            group_info,
        })))
    }

    fn show_success_screen(&self, ui: &mut egui::Ui) -> AppAction {
        crate::ui::helpers::show_group_token_success_screen_with_fee(
            ui,
            "Resume",
            self.base.group_action_id.is_some(),
            self.base.is_unilateral_group_member,
            self.base.group.is_some(),
            &self.base.app_context,
            None,
        )
    }
}

impl ScreenLike for ResumeTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.base.status = OperationStatus::ErrorMessage(message.to_string());
            self.base.error_message = Some(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::Token(TokenResult::ResumedTokens(fee_result)) =
            backend_task_success_result
        {
            self.base.completed_fee_result = Some(fee_result);
            self.base.status = OperationStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        self.base.refresh();
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action;

        // Build a top panel
        if self.base.group_action_id.is_some() {
            action = add_top_panel(
                ctx,
                &self.base.app_context,
                vec![
                    ("Contracts", AppAction::GoToMainScreen),
                    ("Group Actions", AppAction::PopScreen),
                    ("Resume", AppAction::None),
                ],
                vec![],
            );
        } else {
            action = add_top_panel(
                ctx,
                &self.base.app_context,
                vec![
                    ("Tokens", AppAction::GoToMainScreen),
                    (
                        &self.base.identity_token_info.token_alias,
                        AppAction::PopScreen,
                    ),
                    ("Resume", AppAction::None),
                ],
                vec![],
            );
        }

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.base.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.base.app_context);

        island_central_panel(ctx, |ui| {
            if self.base.status == OperationStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Resume Token Contract");
            ui.add_space(10.0);

            if !self.base.render_key_check(ui, &mut action) {
                return;
            }

            if self.base.render_wallet_lock_check(ui) {
                return;
            }

            self.base.render_advanced_header(ui, "Resume");
            self.base.render_key_selection(ui, "Resume");
            self.base.render_public_note(ui, "Resume");
            self.base.render_fee_estimation(ui);

            let button_text = render_group_action_text(
                ui,
                &self.base.group,
                &self.base.identity_token_info,
                "Resume",
                &self.base.group_action_id,
            );

            // Resume button
            if self.base.app_context.is_developer_mode() || !button_text.contains("Test") {
                ui.add_space(10.0);
                let button = egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
                    .fill(DashColors::ACTION_BUTTON_BLUE)
                    .corner_radius(3.0);

                if ui.add(button).clicked() && self.base.confirmation_dialog.is_none() {
                    self.base.confirmation_dialog = Some(ConfirmationDialog::new(
                        "Confirm Resume".to_string(),
                        "Are you sure you want to resume normal token actions for this contract?"
                            .to_string(),
                    ));
                }
            }

            // Show confirmation dialog if it exists
            if self.base.confirmation_dialog.is_some() {
                action |= self
                    .base
                    .show_confirmation_popup(ui, Self::build_resume_task);
            }

            self.base.render_status(ui, "Resuming");
        });

        // Show wallet unlock popup if open
        self.base.show_wallet_unlock_popup(ctx);

        action
    }
}
