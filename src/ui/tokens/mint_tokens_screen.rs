use super::token_action_screen::{
    FILL_IN_THE_FORM, StepCounter, TokenAction, TokenActionCtx, TokenActionScreen,
    parse_identity_id, positive_amount,
};
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::user_role::UserRole;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::theme::DashColors;
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
    pub recipient_identity_id: String,
    known_identities: Vec<QualifiedIdentity>,
    /// The token sends new tokens to a configured identity unless told otherwise.
    has_default_destination: bool,
    /// The token lets the minter name a recipient. When it does not, Platform
    /// refuses any mint that names one, even the configured destination.
    allow_choosing_destination: bool,
    /// New tokens have somewhere to go: a configured destination, or the
    /// token lets the minter choose one.
    can_mint: bool,
}

impl MintAction {
    /// The recipient to mint to: `Some(None)` leaves it to the token's
    /// configured destination; `None` when no valid recipient is known.
    ///
    /// A token that forbids choosing always mints to its configured
    /// destination, so any typed or pre-filled recipient is ignored: Platform
    /// refuses a mint that names one. Co-signing a group mint still matches
    /// the proposal, because Platform compares the resolved recipient (the
    /// configured destination) and leaves the recipient out of the action id.
    fn recipient(&self) -> Option<Option<Identifier>> {
        if !self.allow_choosing_destination {
            return self.has_default_destination.then_some(None);
        }
        if self.recipient_identity_id.trim().is_empty() && self.has_default_destination {
            return Some(None);
        }
        parse_identity_id(&self.recipient_identity_id).map(Some)
    }

    /// The identity the new tokens go to: the entered recipient, or the
    /// token's configured destination when the mint leaves it to the token.
    fn resolved_recipient(&self, ctx: &TokenActionCtx) -> Option<Identifier> {
        match self.recipient()? {
            Some(recipient) => Some(recipient),
            None => ctx
                .info
                .token_config
                .distribution_rules()
                .new_tokens_destination_identity()
                .copied(),
        }
    }

    /// What a group co-signer is told about where the proposed tokens go.
    fn proposed_recipient_text(&self, ctx: &TokenActionCtx) -> String {
        match self.resolved_recipient(ctx) {
            Some(recipient) => format!(
                "Recipient: {recipient}",
                recipient = recipient.to_string(Encoding::Base58)
            ),
            None if self.recipient_identity_id.trim().is_empty() => {
                "This proposal does not name a recipient.".to_string()
            }
            None => format!("Recipient: {}", self.recipient_identity_id),
        }
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

    fn new(info: &IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let rules = info.token_config.distribution_rules();
        let has_default_destination = rules.new_tokens_destination_identity().is_some();
        let allow_choosing_destination = rules.minting_allow_choosing_destination();
        MintAction {
            amount: None,
            amount_input: None,
            recipient_identity_id: String::new(),
            known_identities: super::load_identities_with_banner(app_context),
            has_default_destination,
            allow_choosing_destination,
            can_mint: has_default_destination || allow_choosing_destination,
        }
    }

    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers {
        *config
            .manual_minting_rules()
            .authorized_to_make_change_action_takers()
    }

    fn confirm_message(&self, ctx: &TokenActionCtx) -> String {
        let recipient = self
            .resolved_recipient(ctx)
            .map(|id| id.to_string(Encoding::Base58))
            .unwrap_or_default();
        format!(
            "Are you sure you want to mint {} tokens to {}?",
            self.amount.clone().unwrap_or(Amount::new(0, 0)),
            recipient
        )
    }

    fn render_form(&mut self, ui: &mut Ui, ctx: &TokenActionCtx, step: &mut StepCounter) {
        if !self.can_mint {
            ui.colored_label(
                DashColors::error_color(ui.style().visuals.dark_mode),
                "This token cannot be minted yet because it has no default recipient for new tokens and does not let you choose one. Set a default recipient in the token's configuration, then try again.",
            );
            return;
        }
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
        if ctx.is_group_signing {
            // The co-signer approves the proposal as it stands, so its
            // recipient is shown but cannot be changed.
            ui.add_space(10.0);
            ui.label(
                "You are signing an existing group Mint so you are not allowed to choose the recipient.",
            );
            ui.add_space(5.0);
            ui.label(self.proposed_recipient_text(ctx));
        } else if distribution_rules.minting_allow_choosing_destination()
            || ctx.app_context.user_role().at_least(UserRole::Power)
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

    fn inputs_ready(&self) -> bool {
        self.can_mint && positive_amount(&self.amount).is_some() && self.recipient().is_some()
    }

    fn disabled_reason(&self) -> &'static str {
        if self.can_mint {
            FILL_IN_THE_FORM
        } else {
            "This token cannot be minted until its configuration sets a default recipient for new tokens."
        }
    }

    fn build_action(
        &mut self,
        ctx: &TokenActionCtx,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Option<AppAction> {
        let Some(amount) = positive_amount(&self.amount) else {
            MessageBanner::set_global(
                ctx.app_context.egui_ctx(),
                "Invalid amount",
                MessageType::Error,
            );
            return None;
        };

        let Some(recipient_id) = self.recipient() else {
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
                recipient_id,
                amount,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_contract::QualifiedContract;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Setters;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
    use dash_sdk::dpp::platform_value::BinaryData;

    /// Token info whose token sends new tokens to `destination` by default.
    fn token_info(
        context: &Arc<AppContext>,
        destination: Option<Identifier>,
        allow_choosing: bool,
    ) -> IdentityTokenInfo {
        let identity = QualifiedIdentity::from_bytes(
            &crate::database::test_helpers::basic_legacy_identity_blob(
                [3; 32],
                None,
                context.network,
            ),
        )
        .unwrap();
        let mut config = TokenConfigurationV0::default_most_restrictive();
        config
            .distribution_rules
            .set_new_tokens_destination_identity(destination);
        config
            .distribution_rules
            .set_minting_allow_choosing_destination(allow_choosing);
        IdentityTokenInfo {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity,
            data_contract: QualifiedContract {
                contract: context.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: config.into(),
            token_position: 0,
        }
    }

    fn signing_key() -> IdentityPublicKey {
        IdentityPublicKeyV0 {
            id: 0,
            key_type: KeyType::ECDSA_HASH160,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::CRITICAL,
            read_only: false,
            data: BinaryData::new(vec![1; 20]),
            disabled_at: None,
            contract_bounds: None,
        }
        .into()
    }

    /// The recipient is labelled optional when the token has a default
    /// destination; leaving it empty must mint to that destination instead of
    /// failing with "Invalid receiver".
    #[test]
    fn empty_recipient_mints_to_the_configured_destination() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, Some(Identifier::from([5; 32])), false);
        let ctx = TokenActionCtx {
            info: &info,
            app_context: &context,
            is_group_signing: false,
        };
        let mut action = MintAction::new(&info, &context);
        action.amount = Some(Amount::from_token(&info, 5));

        assert!(
            action.inputs_ready(),
            "the optional recipient may stay empty"
        );
        assert!(
            action
                .confirm_message(&ctx)
                .ends_with(&format!("{}?", Identifier::from([5; 32]))),
            "the confirmation names the configured destination"
        );
        match action.build_action(&ctx, signing_key(), None, None) {
            Some(AppAction::BackendTask(BackendTask::TokenTask(task))) => match *task {
                TokenTask::MintTokens { recipient_id, .. } => assert_eq!(
                    recipient_id, None,
                    "an empty recipient defers to the configured destination"
                ),
                other => panic!("unexpected task {other:?}"),
            },
            other => panic!("expected a mint task, got {other:?}"),
        }
    }

    /// The recipient a built mint task carries; panics when no task is built.
    fn built_recipient(action: &mut MintAction, ctx: &TokenActionCtx) -> Option<Identifier> {
        match action.build_action(ctx, signing_key(), None, None) {
            Some(AppAction::BackendTask(BackendTask::TokenTask(task))) => match *task {
                TokenTask::MintTokens { recipient_id, .. } => recipient_id,
                other => panic!("unexpected task {other:?}"),
            },
            other => panic!("expected a mint task, got {other:?}"),
        }
    }

    /// Platform refuses a mint that names a recipient when the token forbids
    /// choosing one, so a typed or pre-filled recipient must not be sent: the
    /// configured destination applies instead.
    #[test]
    fn a_recipient_is_not_sent_when_the_token_forbids_choosing_one() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let destination = Identifier::from([5; 32]);
        let info = token_info(&context, Some(destination), false);
        let ctx = TokenActionCtx {
            info: &info,
            app_context: &context,
            is_group_signing: false,
        };
        let mut action = MintAction::new(&info, &context);
        action.amount = Some(Amount::from_token(&info, 5));
        action.recipient_identity_id = Identifier::from([6; 32]).to_string(Encoding::Base58);

        assert!(action.inputs_ready());
        assert!(
            action
                .confirm_message(&ctx)
                .ends_with(&format!("{destination}?")),
            "the confirmation names where the tokens really go"
        );
        assert_eq!(built_recipient(&mut action, &ctx), None);
    }

    /// When the token lets the minter choose, a typed recipient is sent even
    /// though a default destination exists.
    #[test]
    fn a_chosen_recipient_is_sent_when_the_token_allows_choosing() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, Some(Identifier::from([5; 32])), true);
        let ctx = TokenActionCtx {
            info: &info,
            app_context: &context,
            is_group_signing: false,
        };
        let chosen = Identifier::from([6; 32]);
        let mut action = MintAction::new(&info, &context);
        action.amount = Some(Amount::from_token(&info, 5));
        action.recipient_identity_id = chosen.to_string(Encoding::Base58);

        assert!(action.inputs_ready());
        assert_eq!(built_recipient(&mut action, &ctx), Some(chosen));
    }

    /// A proposal that leaves the recipient to the token's default
    /// destination shows the co-signer that destination, not an empty name.
    #[test]
    fn a_co_signer_sees_the_default_destination_of_a_proposal_without_recipient() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let destination = Identifier::from([5; 32]);
        let info = token_info(&context, Some(destination), true);
        let ctx = TokenActionCtx {
            info: &info,
            app_context: &context,
            is_group_signing: true,
        };
        let mut action = MintAction::new(&info, &context);
        assert_eq!(
            action.proposed_recipient_text(&ctx),
            format!("Recipient: {}", destination.to_string(Encoding::Base58))
        );

        let proposed = Identifier::from([6; 32]);
        action.recipient_identity_id = proposed.to_string(Encoding::Base58);
        assert_eq!(
            action.proposed_recipient_text(&ctx),
            format!("Recipient: {}", proposed.to_string(Encoding::Base58))
        );

        let without_destination = token_info(&context, None, true);
        let ctx = TokenActionCtx {
            info: &without_destination,
            app_context: &context,
            is_group_signing: true,
        };
        let action = MintAction::new(&without_destination, &context);
        assert_eq!(
            action.proposed_recipient_text(&ctx),
            "This proposal does not name a recipient."
        );
    }

    /// Without a configured destination the recipient stays required.
    #[test]
    fn empty_recipient_is_rejected_without_a_destination() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, None, true);
        let mut action = MintAction::new(&info, &context);
        action.amount = Some(Amount::from_token(&info, 5));

        assert!(!action.inputs_ready());
    }

    /// A token with no destination that does not let the minter choose one
    /// cannot be minted at all, so the form must not offer to.
    #[test]
    fn a_token_without_any_destination_cannot_be_minted() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, None, false);
        let mut action = MintAction::new(&info, &context);
        action.amount = Some(Amount::from_token(&info, 5));
        action.recipient_identity_id = Identifier::from([6; 32]).to_string(Encoding::Base58);

        assert!(!action.inputs_ready());
    }

    /// There is no form to fill in on a token that cannot be minted, so the
    /// disabled button must not tell the user to fill one in.
    #[test]
    fn the_disabled_mint_button_explains_why_the_token_cannot_be_minted() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());

        let unmintable = MintAction::new(&token_info(&context, None, false), &context);
        assert_ne!(unmintable.disabled_reason(), FILL_IN_THE_FORM);

        let mintable = MintAction::new(&token_info(&context, None, true), &context);
        assert_eq!(mintable.disabled_reason(), FILL_IN_THE_FORM);
    }
}
