//! Token detail inside the Identity Hub Tokens tab: what the identity holds,
//! whether it can be sent, what waits to be claimed, and who issued it, with
//! copyable IDs. Opened from a row; "Back to tokens" returns to the list.
//!
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` frame T5.

use super::tokens_tab::{
    CHECK_NOW_LABEL, DISABLED_NOT_SYNCED, MANAGE_LABEL, RowContext, STOP_TRACKING_LABEL,
    manage_action, open_stop_tracking_dialog, render_row_actions, render_row_status,
};
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::identity_name::shorten_id;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::state::identity_tokens::{IdentityTokensState, TokenRow, row_status};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Spacing, Typography};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{RichText, Ui};
use std::sync::Arc;

pub const BACK_LABEL: &str = "‹ Back to tokens";
pub const ABOUT_HEADING: &str = "About this token";
pub const TOKEN_ID_LABEL: &str = "Token ID";
pub const CONTRACT_LABEL: &str = "Contract";
pub const ISSUED_BY_LABEL: &str = "Issued by";
pub const SMALLEST_UNIT_LABEL: &str = "Smallest unit";
pub const CAN_BE_SENT_LABEL: &str = "Can be sent";
pub const REWARDS_LABEL: &str = "Rewards";
pub const COPY_LABEL: &str = "Copy";
pub const CAN_BE_SENT_YES: &str = "Yes, to any identity.";
pub const CAN_BE_SENT_NO: &str = "No. This token cannot be sent to other identities.";
pub const REWARDS_OVER_TIME: &str = "Paid out to holders over time. Claim them whenever you like.";
pub const REWARDS_NONE: &str = "This token pays no rewards.";
pub const ISSUED_BY_YOU: &str = "(you)";
pub const ISSUED_BY_SOMEONE_ELSE: &str = "(someone else)";

/// "Held by {identity}"
pub fn held_by_text(identity: &str) -> String {
    format!("Held by {identity}")
}

/// "Every identity can claim {amount} once."
pub fn once_claim_text(amount: &crate::model::amount::Amount) -> String {
    format!("Every identity can claim {amount} once.")
}

/// The detail page for `row`. Clicks may return the action they cause.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    row: &TokenRow,
    state: &mut IdentityTokensState,
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let identity_label = app_context.identity_display_label(identity);
    let context = RowContext {
        synced: app_context.connection_status().masternodes_ready(),
        view_only: identity.private_keys.is_empty(),
        identity_label: &identity_label,
    };
    if ui.link(BACK_LABEL).clicked() {
        state.detail = None;
    }
    ui.add_space(Spacing::SM);
    egui::ScrollArea::vertical()
        .id_salt("identity_token_detail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            super::home::card(ui, |ui| {
                ui.horizontal(|ui| {
                    super::avatar::paint_identity_monogram(
                        ui,
                        56.0,
                        HeroIdentityKind::User,
                        row.name.chars().next(),
                        DashColors::pastel_gradient(row.pair.token_id.as_bytes()[0] as usize),
                    );
                    ui.add_space(Spacing::SM);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&row.name).size(Typography::SCALE_XL).strong());
                        ui.label(
                            RichText::new(held_by_text(&identity_label))
                                .color(DashColors::text_secondary(dark)),
                        );
                    });
                });
                ui.add_space(Spacing::MD);
                if let Some(balance) = row.balance {
                    ui.label(
                        RichText::new(row.amount(balance).to_string())
                            .size(Typography::SCALE_XXL)
                            .monospace(),
                    );
                }
                let status = row_status(row, context.view_only);
                render_row_status(ui, app_context, row, &status, context, &mut action, state);
                ui.add_space(Spacing::SM);
                ui.horizontal_wrapped(|ui| {
                    action |= render_row_actions(ui, app_context, row, context);
                });
            });
            ui.add_space(Spacing::MD);
            super::home::card(ui, |ui| {
                ui.strong(ABOUT_HEADING);
                ui.add_space(Spacing::XS);
                egui::Grid::new("identity_token_facts")
                    .num_columns(2)
                    .spacing(egui::vec2(Spacing::MD, Spacing::SM))
                    .show(ui, |ui| {
                        fact_label(ui, TOKEN_ID_LABEL);
                        copyable(ui, &row.token_id_base58(), &row.token_id_base58());
                        ui.end_row();

                        fact_label(ui, CONTRACT_LABEL);
                        let contract = row.contract_id.to_string(Encoding::Base58);
                        copyable(ui, &shorten_id(&contract), &contract);
                        ui.end_row();

                        fact_label(ui, ISSUED_BY_LABEL);
                        let issuer = row.issuer_id.to_string(Encoding::Base58);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(shorten_id(&issuer)).monospace());
                            ui.label(
                                RichText::new(if row.issued_by_me {
                                    ISSUED_BY_YOU
                                } else {
                                    ISSUED_BY_SOMEONE_ELSE
                                })
                                .color(DashColors::text_secondary(dark)),
                            );
                        });
                        ui.end_row();

                        fact_label(ui, SMALLEST_UNIT_LABEL);
                        ui.label(row.amount(1).to_string());
                        ui.end_row();

                        fact_label(ui, CAN_BE_SENT_LABEL);
                        ui.label(if row.transferable {
                            CAN_BE_SENT_YES
                        } else {
                            CAN_BE_SENT_NO
                        });
                        ui.end_row();

                        fact_label(ui, REWARDS_LABEL);
                        ui.label(rewards_text(row));
                        ui.end_row();
                    });
                ui.add_space(Spacing::MD);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            context.synced,
                            ComponentStyles::secondary_button(CHECK_NOW_LABEL, dark),
                        )
                        .disabled_tooltip(DISABLED_NOT_SYNCED)
                        .clicked()
                    {
                        action |= state.check_pair_action(app_context, row.pair);
                    }
                    if ComponentStyles::add_secondary_button(ui, STOP_TRACKING_LABEL, dark)
                        .clicked()
                    {
                        open_stop_tracking_dialog(state, row, &identity_label);
                    }
                    if row.has_admin_permission && ui.link(MANAGE_LABEL).clicked() {
                        action |= manage_action(app_context, row);
                    }
                });
            });
        });
    action
}

/// What the token pays its holders, in plain words.
fn rewards_text(row: &TokenRow) -> String {
    match &row.once_claim {
        Some(once) => match once.claimed_at_ms {
            Some(claimed_at_ms) => super::tokens_tab::claimed_on_text(
                &crate::model::datetime::instant_from_unix_millis(claimed_at_ms)
                    .map(crate::model::datetime::local_date)
                    .unwrap_or_default(),
            ),
            None => once_claim_text(&row.amount(once.amount)),
        },
        None if row.can_claim => REWARDS_OVER_TIME.to_string(),
        None => REWARDS_NONE.to_string(),
    }
}

fn fact_label(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).color(DashColors::text_secondary(ui.visuals().dark_mode)));
}

/// `shown` with a Copy button that copies `full`.
fn copyable(ui: &mut Ui, shown: &str, full: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(shown).monospace());
        if ui.small_button(COPY_LABEL).clicked() {
            ui.ctx().copy_text(full.to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_sentences_are_complete() {
        for sentence in [
            CAN_BE_SENT_YES,
            CAN_BE_SENT_NO,
            REWARDS_OVER_TIME,
            REWARDS_NONE,
        ] {
            assert!(
                sentence.ends_with('.'),
                "'{sentence}' must be a complete sentence"
            );
        }
        assert!(once_claim_text(&crate::model::amount::Amount::new(1, 0)).ends_with('.'));
        assert!(
            !held_by_text("Alice").ends_with('.'),
            "a caption, not a sentence"
        );
    }

    #[test]
    fn smallest_unit_follows_the_token_decimals() {
        use crate::model::token::IdentityTokenIdentifier;
        use dash_sdk::dpp::data_contract::TokenConfiguration;
        use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
        use dash_sdk::platform::Identifier;
        let row = TokenRow {
            pair: IdentityTokenIdentifier {
                identity_id: Identifier::from([1; 32]),
                token_id: Identifier::from([2; 32]),
            },
            name: "DCF".into(),
            contract_id: Identifier::from([3; 32]),
            issuer_id: Identifier::from([4; 32]),
            token_position: 0,
            token_config: TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive()),
            balance: Some(5),
            issued_by_me: false,
            transferable: true,
            can_claim: false,
            once_claim: None,
            has_admin_permission: false,
            check_failed: false,
        };
        assert_eq!(row.amount(1).to_string(), "0.00000001 DCF");
        assert_eq!(rewards_text(&row), REWARDS_NONE);
    }
}
