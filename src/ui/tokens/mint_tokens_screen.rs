use super::token_action_screen::{StepCounter, TokenAction, TokenActionCtx, TokenActionScreen};
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::identity_selector::IdentitySelector;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::Ui;
use std::sync::Arc;

/// Mints new tokens to a recipient (defaulting to the issuer).
pub type MintTokensScreen = TokenActionScreen<MintAction>;

pub struct MintAction {
    pub amount: Option<Amount>,
    amount_input: Option<AmountInput>,
    recipient_identity_id: String,
    known_identities: Vec<QualifiedIdentity>,
}

impl MintAction {
    fn render_amount_input(&mut self, ui: &mut Ui, ctx: &TokenActionCtx) {
        let amount_input = self.amount_input.get_or_insert_with(|| {
            let token_amount = Amount::from_token(ctx.info, 0);
            AmountInput::new(token_amount).with_label("Amount to Mint:")
        });
        let response = amount_input.show(ui).inner;
        response.update(&mut self.amount);
    }

    fn render_recipient_input(&mut self, ui: &mut Ui, ctx: &TokenActionCtx) {
        ui.add(
            IdentitySelector::new(
                "mint_recipient_selector",
                &mut self.recipient_identity_id,
                &self.known_identities,
            )
            .width(300.0)
            .label("Recipient:")
            .exclude(&[ctx.info.identity.identity.id()]),
        );
    }
}

impl TokenAction for MintAction {
    const VERB: &'static str = "Mint";
    const PAGE_HEADING: &'static str = "Mint Tokens";
    const PROGRESS: &'static str = "Minting tokens...";
    const CONFIRM_TITLE: &'static str = "Confirm Mint";

    fn new(_info: &IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        MintAction {
            amount: None,
            amount_input: None,
            recipient_identity_id: String::new(),
            known_identities: super::load_identities_with_banner(app_context),
        }
    }

    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers {
        *config
            .manual_minting_rules()
            .authorized_to_make_change_action_takers()
    }

    fn confirm_message(&self, _ctx: &TokenActionCtx) -> String {
        format!(
            "Are you sure you want to mint {} tokens to {}?",
            self.amount.clone().unwrap_or(Amount::new(0, 0)),
            self.recipient_identity_id
        )
    }

    fn render_form(&mut self, ui: &mut Ui, ctx: &TokenActionCtx, step: &mut StepCounter) {
        let n = step.advance();
        ui.heading(format!("{}. Amount to mint", n));
        ui.add_space(5.0);
        if ctx.is_group_signing {
            ui.label(
                "You are signing an existing group Mint so you are not allowed to choose the amount.",
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

        let distribution_rules = ctx.info.token_config.distribution_rules();
        if distribution_rules.minting_allow_choosing_destination()
            || ctx.app_context.is_developer_mode()
        {
            let destination_optional = distribution_rules
                .new_tokens_destination_identity()
                .is_some();

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            let n = step.advance();
            if destination_optional {
                ui.heading(format!("{}. Recipient identity (optional)", n));
            } else {
                ui.heading(format!("{}. Recipient identity (required)", n));
            }
            ui.add_space(5.0);
            self.render_recipient_input(ui, ctx);
        }
    }

    fn build_action(
        &mut self,
        ctx: &TokenActionCtx,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Option<AppAction> {
        if self.amount.is_none() || self.amount == Some(Amount::new(0, 0)) {
            MessageBanner::set_global(
                ctx.app_context.egui_ctx(),
                "Invalid amount",
                MessageType::Error,
            );
            return None;
        }

        let Ok(recipient_id) = Identifier::from_string_try_encodings(
            &self.recipient_identity_id,
            &[Encoding::Base58, Encoding::Hex],
        ) else {
            MessageBanner::set_global(
                ctx.app_context.egui_ctx(),
                "Invalid receiver",
                MessageType::Error,
            );
            return None;
        };

        Some(AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::MintTokens {
                sending_identity: ctx.info.identity.clone(),
                data_contract: ctx.data_contract(),
                token_position: ctx.info.token_position,
                signing_key,
                public_note,
                recipient_id: Some(recipient_id),
                amount: self.amount.clone().unwrap_or(Amount::new(0, 0)).value(),
                group_info,
            },
        ))))
    }

    fn success_fee(result: &BackendTaskSuccessResult) -> Option<FeeResult> {
        match result {
            BackendTaskSuccessResult::MintedTokens(fee) => Some(fee.clone()),
            _ => None,
        }
    }
}
