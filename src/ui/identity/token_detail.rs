//! Token detail inside the Identity Hub Tokens tab: what the identity holds,
//! whether it can be sent, what waits to be claimed, and who issued it, with
//! copyable IDs. Opened from a row; "Back to tokens" returns to the list.
//!
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` frame T5.

use super::token_copy::*;
use super::tokens_tab::{
    DETAIL_MONOGRAM_SIZE, RowContext, manage_action, open_stop_tracking_dialog,
    paint_token_monogram, refresh_block_reason, render_row_actions, render_row_status,
};
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::identity_name::shorten_id;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::state::identity_tokens::{IdentityTokensState, TokenRow, row_status};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Spacing, Typography};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{RichText, Ui};
use std::sync::Arc;

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
    let context = RowContext::new(app_context, identity);
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
                    paint_token_monogram(ui, row, DETAIL_MONOGRAM_SIZE);
                    ui.add_space(Spacing::SM);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(row.name())
                                .size(Typography::SCALE_XL)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(held_by_text(&context.identity_label))
                                .color(DashColors::text_secondary(dark)),
                        );
                    });
                });
                ui.add_space(Spacing::MD);
                if let Some(balance) = row.balance() {
                    ui.label(
                        RichText::new(row.amount(balance).to_string())
                            .size(Typography::SCALE_XXL)
                            .monospace(),
                    );
                }
                let status = row_status(row, context.view_only);
                action |= render_row_status(ui, app_context, row, &status, &context, state);
                ui.add_space(Spacing::SM);
                ui.horizontal_wrapped(|ui| {
                    action |= render_row_actions(ui, app_context, row, &context);
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
                        let contract = row.token.data_contract_id.to_string(Encoding::Base58);
                        copyable(ui, &shorten_id(&contract), &contract);
                        ui.end_row();

                        fact_label(ui, ISSUED_BY_LABEL);
                        let issuer = row.issuer_id.to_string(Encoding::Base58);
                        ui.horizontal(|ui| {
                            ui.label(if row.issued_by_me {
                                ISSUED_BY_YOU
                            } else {
                                ISSUED_BY_SOMEONE_ELSE
                            });
                            ui.label(
                                RichText::new(shorten_id(&issuer))
                                    .monospace()
                                    .color(DashColors::text_secondary(dark)),
                            );
                        });
                        ui.end_row();

                        fact_label(ui, SMALLEST_UNIT_LABEL);
                        ui.label(row.amount(1).to_string());
                        ui.end_row();

                        fact_label(ui, CAN_BE_SENT_LABEL);
                        ui.label(if row.transferable() {
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
                let refreshing = state.refreshing(app_context);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            context.synced && !refreshing,
                            ComponentStyles::secondary_button(CHECK_NOW_LABEL, dark),
                        )
                        .disabled_tooltip(refresh_block_reason(context.synced))
                        .clicked()
                    {
                        action |= state.check_pair_action(app_context, row.pair());
                    }
                    if ComponentStyles::add_secondary_button(ui, STOP_TRACKING_LABEL, dark)
                        .clicked()
                    {
                        open_stop_tracking_dialog(state, row, &context.identity_label);
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
    match (&row.once_claim, row.claimed_on_date()) {
        (Some(_), Some(date)) => claimed_on_text(&date),
        (Some(once), None) => once_claim_text(&row.amount(once.amount)),
        (None, _) if row.can_claim() => REWARDS_OVER_TIME.to_string(),
        (None, _) => REWARDS_NONE.to_string(),
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
    use crate::model::token::{
        IdentityTokenAvailableActions, IdentityTokenMaybeBalanceWithActions,
    };
    use crate::ui::state::identity_tokens::OnceClaim;
    use dash_sdk::dpp::data_contract::TokenConfiguration;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::platform::Identifier;

    fn row() -> TokenRow {
        TokenRow {
            token: IdentityTokenMaybeBalanceWithActions {
                token_id: Identifier::from([2; 32]),
                token_alias: "DCF".into(),
                token_config: TokenConfiguration::V0(
                    TokenConfigurationV0::default_most_restrictive(),
                ),
                identity_id: Identifier::from([1; 32]),
                identity_name: None,
                balance: Some(5),
                estimated_unclaimed_rewards: None,
                data_contract_id: Identifier::from([3; 32]),
                token_position: 0,
                available_actions: IdentityTokenAvailableActions::default(),
            },
            issuer_id: Identifier::from([4; 32]),
            issued_by_me: false,
            once_claim: None,
            has_admin_permission: false,
            check_failed: false,
        }
    }

    #[test]
    fn smallest_unit_follows_the_token_decimals() {
        let row = row();
        assert_eq!(row.amount(1).to_string(), "0.00000001 DCF");
        assert_eq!(rewards_text(&row), REWARDS_NONE);
    }

    #[test]
    fn rewards_fact_names_the_once_claim_or_its_date() {
        let mut row = row();
        row.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: None,
        });
        assert_eq!(rewards_text(&row), once_claim_text(&row.amount(100)));
        row.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: Some(1_700_000_000_000),
        });
        assert!(rewards_text(&row).starts_with("Claimed on "));
        let mut perpetual = self::row();
        perpetual.token.available_actions.can_claim = true;
        assert_eq!(rewards_text(&perpetual), REWARDS_OVER_TIME);
    }
}
