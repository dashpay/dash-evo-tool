use super::token_action_screen::{StepCounter, TokenAction, TokenActionCtx, TokenActionScreen};
use super::tokens_screen::{IdentityTokenIdentifier, IdentityTokenInfo};
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::Ui;
use std::sync::Arc;

/// Burns a chosen amount of the caller's tokens.
pub type BurnTokensScreen = TokenActionScreen<BurnAction>;

pub struct BurnAction {
    pub amount: Option<Amount>,
    amount_input: Option<AmountInput>,
    /// Maximum amount the user can burn based on their balance.
    max_amount: Option<u64>,
}

impl BurnAction {
    fn render_amount_input(&mut self, ui: &mut Ui, ctx: &TokenActionCtx) {
        let max_amount = self.max_amount;
        let amount_input = self.amount_input.get_or_insert_with(|| {
            let token_amount = Amount::from_token(ctx.info, 0);
            let mut input = AmountInput::new(token_amount).with_label("Amount:");
            if max_amount.is_some() {
                input.set_show_max_button(true);
                input.set_max_amount(max_amount);
            }
            input
        });
        let amount_response = amount_input.show(ui).inner;
        amount_response.update(&mut self.amount);
    }
}

impl TokenAction for BurnAction {
    const VERB: &'static str = "Burn";
    const PAGE_HEADING: &'static str = "Burn Tokens";
    const PROGRESS: &'static str = "Burning tokens...";
    const CONFIRM_TITLE: &'static str = "Confirm Burn";
    const DANGER: bool = true;

    fn new(info: &IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let max_amount = match app_context.identity_token_balances() {
            Ok(balances) => {
                let key = IdentityTokenIdentifier {
                    identity_id: info.identity.identity.id(),
                    token_id: info.token_id,
                };
                balances.get(&key).map(|itb| itb.balance)
            }
            Err(_) => None,
        };
        BurnAction {
            amount: None,
            amount_input: None,
            max_amount,
        }
    }

    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers {
        *config
            .manual_burning_rules()
            .authorized_to_make_change_action_takers()
    }

    fn confirm_message(&self, _ctx: &TokenActionCtx) -> String {
        format!(
            "Are you sure you want to burn {}?",
            self.amount.clone().unwrap_or(Amount::new(0, 0))
        )
    }

    fn render_form(&mut self, ui: &mut Ui, ctx: &TokenActionCtx, step: &mut StepCounter) {
        let n = step.advance();
        ui.heading(format!("{}. Amount to burn", n));
        ui.add_space(5.0);
        if ctx.is_group_signing {
            ui.label(
                "You are signing an existing group Burn so you are not allowed to choose the amount.",
            );
            ui.add_space(5.0);
            ui.label(format!(
                "Amount: {}",
                self.amount
                    .as_ref()
                    .map(|a| a.to_string())
                    .unwrap_or_default()
            ));
        } else {
            self.render_amount_input(ui, ctx);
        }
    }

    fn build_action(
        &mut self,
        ctx: &TokenActionCtx,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Option<AppAction> {
        let amount = match self.amount.as_ref() {
            Some(amount) if amount.value() > 0 => amount.value(),
            _ => {
                MessageBanner::set_global(
                    ctx.app_context.egui_ctx(),
                    "Please enter a valid amount greater than 0.",
                    MessageType::Error,
                );
                return None;
            }
        };

        Some(AppAction::BackendTasks(
            vec![
                BackendTask::TokenTask(Box::new(TokenTask::BurnTokens {
                    owner_identity: ctx.info.identity.clone(),
                    data_contract: ctx.data_contract(),
                    token_position: ctx.info.token_position,
                    signing_key,
                    public_note,
                    amount,
                    group_info,
                })),
                BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
            ],
            BackendTasksExecutionMode::Sequential,
        ))
    }

    fn success_fee(result: &BackendTaskSuccessResult) -> Option<FeeResult> {
        match result {
            BackendTaskSuccessResult::BurnedTokens(fee) => Some(fee.clone()),
            _ => None,
        }
    }
}
