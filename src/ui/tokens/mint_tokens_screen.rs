use super::token_action_screen::{StepCounter, TokenAction, TokenActionCtx, TokenActionScreen};
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::MintRecipient;
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
    /// The recipient the minter entered: `Ok(None)` when the field is empty or
    /// the token does not let the minter choose, `Err` when it does not parse.
    fn chosen_recipient(&self, ctx: &TokenActionCtx) -> Result<Option<Identifier>, ()> {
        let entered = self.recipient_identity_id.trim();
        if entered.is_empty()
            || !ctx
                .info
                .token_config
                .distribution_rules()
                .minting_allow_choosing_destination()
        {
            return Ok(None);
        }
        Identifier::from_string_try_encodings(entered, &[Encoding::Base58, Encoding::Hex])
            .map(Some)
            .map_err(|_| ())
    }

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
            .with_context(ctx.app_context)
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

    fn confirm_message(&self, ctx: &TokenActionCtx) -> String {
        let amount = self.amount.clone().unwrap_or(Amount::new(0, 0));
        match self.chosen_recipient(ctx) {
            Ok(chosen) => {
                match MintRecipient::resolve(ctx.info.token_config.distribution_rules(), chosen) {
                    MintRecipient::Chosen(recipient) => format!(
                        "Are you sure you want to mint {amount} tokens to {recipient}?",
                        recipient = recipient.to_string(Encoding::Base58)
                    ),
                    MintRecipient::ContractDestination(recipient) => format!(
                        "Are you sure you want to mint {amount} tokens to this token's destination identity {recipient}?",
                        recipient = recipient.to_string(Encoding::Base58)
                    ),
                    MintRecipient::Missing => format!(
                        "Are you sure you want to mint {amount} tokens? This token has no destination identity for new tokens."
                    ),
                }
            }
            Err(()) => format!(
                "Are you sure you want to mint {amount} tokens to {recipient}?",
                recipient = self.recipient_identity_id
            ),
        }
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
        if !distribution_rules.minting_allow_choosing_destination() {
            ui.add_space(10.0);
            match distribution_rules.new_tokens_destination_identity() {
                Some(destination) => ui.label(format!(
                    "New tokens go to this token's destination identity {destination}.",
                    destination = destination.to_string(Encoding::Base58)
                )),
                None => ui.label(
                    "This token has no destination identity for new tokens, so it cannot be minted.",
                ),
            };
        } else {
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

        let Ok(chosen) = self.chosen_recipient(ctx) else {
            MessageBanner::set_global(
                ctx.app_context.egui_ctx(),
                "Invalid receiver",
                MessageType::Error,
            );
            return None;
        };
        let rules = ctx.info.token_config.distribution_rules();
        let recipient = MintRecipient::resolve(rules, chosen);
        if recipient == MintRecipient::Missing {
            let message = if rules.minting_allow_choosing_destination() {
                "Choose the identity that receives the new tokens, then try again."
            } else {
                "This token has no destination identity for new tokens, so it cannot be minted."
            };
            MessageBanner::set_global(ctx.app_context.egui_ctx(), message, MessageType::Error);
            return None;
        }

        Some(AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::MintTokens {
                sending_identity: ctx.info.identity.clone(),
                data_contract: ctx.data_contract(),
                token_position: ctx.info.token_position,
                signing_key,
                public_note,
                recipient_id: recipient.transition_recipient(),
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
