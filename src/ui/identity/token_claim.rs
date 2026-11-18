//! Claim form embedded in the identity hub; backend work belongs to its owner.
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::ui::helpers::{TransactionType, add_key_chooser, format_key_label};
use crate::ui::state::identity_tokens::TokenRow;
use crate::ui::theme::{ComponentStyles, ResponseExt, Spacing};
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::platform::IdentityPublicKey;
use std::sync::Arc;

/// The user's decision; the selected key and distribution are frozen for confirmation.
#[derive(Clone)]
pub enum ClaimDecision {
    Confirmed(TokenDistributionType, IdentityPublicKey),
    Canceled,
}

/// Response from the claim dialog.
#[derive(Clone)]
pub struct ClaimResponse {
    pub decision: Option<ClaimDecision>,
}

impl ComponentResponse for ClaimResponse {
    type DomainType = ClaimDecision;
    fn has_changed(&self) -> bool {
        self.decision.is_some()
    }
    fn is_valid(&self) -> bool {
        true
    }
    fn changed_value(&self) -> &Option<ClaimDecision> {
        &self.decision
    }
    fn error_message(&self) -> Option<&str> {
        None
    }
}

/// Identity-scoped claim form using the shared key chooser and confirmation chrome.
pub struct TokenClaimDialog {
    app_context: Arc<AppContext>,
    identity: QualifiedIdentity,
    row: TokenRow,
    offered: Vec<TokenDistributionType>,
    selected: Option<TokenDistributionType>,
    key: Option<IdentityPublicKey>,
    estimate: Option<u64>,
    estimate_pending: bool,
    confirmation: Option<ConfirmationDialog>,
    navigation: Option<Box<AppAction>>,
}

impl TokenClaimDialog {
    /// Open the claim form for one identity and its eligible distributions.
    pub fn new(
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
        row: &TokenRow,
        offered: Vec<TokenDistributionType>,
    ) -> Self {
        Self {
            app_context: app_context.clone(),
            identity: identity.clone(),
            row: row.clone(),
            selected: offered.first().copied(),
            estimate_pending: offered.contains(&TokenDistributionType::Perpetual),
            offered,
            key: None,
            estimate: None,
            confirmation: None,
            navigation: None,
        }
    }

    /// Accept an estimate only through the owner's matching request.
    pub fn estimated(&mut self, amount: Option<u64>) {
        self.estimate = amount;
        self.estimate_pending = false;
    }

    /// Navigation requested by the shared key chooser (for example, Add key).
    pub fn take_navigation(&mut self) -> AppAction {
        self.navigation
            .take()
            .map(|action| *action)
            .unwrap_or(AppAction::None)
    }

    fn amount_text(&self) -> String {
        match self.selected {
            Some(TokenDistributionType::OncePerIdentity) => self.row.once_claim.as_ref()
                .map(|once| format!("You can claim {amount} once.", amount = self.row.amount(once.amount)))
                .unwrap_or_default(),
            Some(TokenDistributionType::Perpetual) if self.estimate_pending => "Checking available rewards…".into(),
            Some(TokenDistributionType::Perpetual) => match self.estimate {
                Some(0) => "No rewards are available yet. Check again later.".into(),
                Some(amount) => format!("Estimated rewards: {amount}. The amount received may differ.", amount = self.row.amount(amount)),
                None => "The reward estimate is unavailable. The network will check what you can claim.".into(),
            },
            _ => "The network will check which scheduled rewards are still available. An amount estimate is unavailable.".into(),
        }
    }
}

impl Component for TokenClaimDialog {
    type DomainType = ClaimDecision;
    type Response = ClaimResponse;

    fn current_value(&self) -> Option<ClaimDecision> {
        None
    }

    fn show(&mut self, ui: &mut egui::Ui) -> egui::InnerResponse<ClaimResponse> {
        if let Some(confirmation) = &mut self.confirmation {
            let result = confirmation.show(ui);
            let decision = match result.inner.dialog_response {
                Some(ConfirmationStatus::Confirmed) => self
                    .selected
                    .zip(self.key.clone())
                    .map(|(kind, key)| ClaimDecision::Confirmed(kind, key)),
                Some(ConfirmationStatus::Canceled) => {
                    self.confirmation = None;
                    None
                }
                None => None,
            };
            return egui::InnerResponse::new(ClaimResponse { decision }, result.response);
        }
        let ctx = ui.ctx().clone();
        let mut decision = None;
        let modal = modal_chrome(
            &ctx,
            ModalChromeConfig {
                title: "Claim token rewards".into(),
                overlay_id: egui::Id::new("identity_token_claim"),
                overlay_order: egui::Order::Middle,
                window_order: egui::Order::Foreground,
                resizable: true,
                show_close_button: true,
                blocks_input: true,
                inner_margin: Spacing::MD as i8,
            },
            |ui| {
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.7).show(ui, |ui| {
                ui.strong(self.row.name());
                ui.label(format!("Claim for {identity}.", identity = self.app_context.identity_display_label(&self.identity)));
                ui.add_space(Spacing::SM);
                ui.label("Distribution:");
                for kind in &self.offered {
                    ui.radio_value(&mut self.selected, Some(*kind), distribution_label(*kind));
                }
                ui.label(self.amount_text());
                ui.add_space(Spacing::SM);
                let navigation = add_key_chooser(ui, &self.app_context, &self.identity, &mut self.key, TransactionType::TokenClaim);
                if !matches!(navigation, AppAction::None) { self.navigation = Some(Box::new(navigation)); }
                ui.add_space(Spacing::SM);
                let fee = format_credits_as_dash(self.app_context.fee_estimator().estimate_document_batch(1));
                ui.label(format!("Estimated fee: {fee}."));
                let available = self.selected.is_some()
                    && !(self.selected == Some(TokenDistributionType::Perpetual)
                        && (self.estimate_pending || self.estimate == Some(0)));
                let enabled = available && self.key.is_some() && self.app_context.connection_status().masternodes_ready();
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ComponentStyles::add_primary_button_enabled(ui, enabled, "Review claim")
                            .disabled_tooltip("Wait for the network and rewards check, then select a signing key.").clicked()
                            && let Some((kind, key)) = self.selected.zip(self.key.as_ref())
                        {
                            let token = self.row.name();
                            let distribution = distribution_label(kind);
                            let identity = self.app_context.identity_display_label(&self.identity);
                            let key = format_key_label(key);
                            let amount = self.amount_text();
                            self.confirmation = Some(ConfirmationDialog::new("Confirm claim", format!(
                                "Claim {token} rewards for {identity} from {distribution}?\n\n{amount}\n\nSigning key: {key}.\nEstimated fee: {fee}."
                            )).confirm_text(Some("Confirm claim")).blocks_input(true));
                        }
                        if ComponentStyles::add_secondary_button(ui, "Cancel", ui.visuals().dark_mode).clicked() {
                            decision = Some(ClaimDecision::Canceled);
                        }
                    });
                });
            });
            },
        );
        if modal.closed_via_x
            || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            decision = Some(ClaimDecision::Canceled);
        }
        let response = modal
            .window_response
            .unwrap_or_else(|| ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()));
        egui::InnerResponse::new(ClaimResponse { decision }, response)
    }
}

fn distribution_label(kind: TokenDistributionType) -> &'static str {
    match kind {
        TokenDistributionType::OncePerIdentity => "Once per identity",
        TokenDistributionType::Perpetual => "Perpetual",
        TokenDistributionType::PreProgrammed => "Pre-programmed",
    }
}
