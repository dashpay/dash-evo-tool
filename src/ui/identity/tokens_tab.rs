//! Identity Hub Tokens tab: balances and rewards for the selected identity.
//! Sending, claiming and every administrative action stay in Tokens › My Tokens.
//!
//! Copy lives in `token_copy.rs`; state in `ui/state/identity_tokens.rs`.
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` (frames T1–T3, T9).

use super::home::HomeState;
use super::token_copy::*;
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::identity_name::shorten_id;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::spv_status::SpvStatus;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::copyable_id::copy_with_confirmation;
use crate::ui::components::inline_notice::InlineNotice;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::components::pill::accent_pill;
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::state::identity_tokens::{
    IdentityTokensState, RowStatus, TokenRow, reward_beside_balance, row_is_listed, sort_rows,
};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Spacing, Typography};
use crate::ui::{MessageType, RootScreenType, ScreenType};
use egui::containers::menu::MenuButton;
use egui::{self, RichText, Ui};
use std::sync::Arc;

/// A wide row lays the balance and the actions out as columns beside the
/// name; a narrow one stacks them under it.
const WIDE_ROW_MIN_WIDTH: f32 = 640.0;
const ACTIONS_COLUMN_WIDTH: f32 = 220.0;
const BALANCE_COLUMN_WIDTH: f32 = 200.0;
const NAME_COLUMN_MIN_WIDTH: f32 = 120.0;
/// The token monogram on a row.
const ROW_MONOGRAM_SIZE: f32 = 36.0;

/// What every row needs to know about the identity and the network.
struct RowContext {
    /// Balance reads may go out: see [`IdentityTokensState::can_read`].
    synced: bool,
    /// Why a read control is disabled.
    block_reason: &'static str,
    /// The identity can sign nothing on this device.
    view_only: bool,
    identity_label: String,
}

impl RowContext {
    fn new(app_context: &AppContext, identity: &QualifiedIdentity) -> Self {
        Self {
            synced: IdentityTokensState::can_read(app_context),
            block_reason: refresh_block_reason(app_context),
            view_only: identity.private_keys.is_empty(),
            identity_label: app_context.identity_display_label(identity),
        }
    }
}

/// The token's monogram: its initial on a hue derived from its ID.
fn paint_token_monogram(ui: &mut Ui, row: &TokenRow, size: f32) {
    super::avatar::paint_identity_monogram(
        ui,
        size,
        HeroIdentityKind::User,
        row.name().chars().next(),
        DashColors::pastel_gradient(row.token.token_id.as_bytes()[0] as usize),
    );
}

/// The tab's content below the hub header. Data loads are not part of
/// rendering: see [`load_action`]. Clicks may return the action they cause.
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: Option<&QualifiedIdentity>,
    state: &mut IdentityTokensState,
    home: &mut HomeState,
) -> AppAction {
    let Some(identity) = identity else {
        return AppAction::None;
    };
    let mut action = AppAction::None;
    let rows = state.rows(app_context, identity);

    action |= render_header(ui, app_context, state);
    ui.add_space(Spacing::XS);
    ui.label(
        RichText::new(SUBTITLE)
            .small()
            .color(DashColors::text_secondary(ui.visuals().dark_mode)),
    );
    ui.add_space(Spacing::SM);
    render_state_bar(ui, state, home);
    action |= render_failure(ui, app_context, state);
    let row_load_failed = render_row_load_failure(ui, state);

    if rows.is_empty() {
        // An unreadable store says nothing about whether tokens exist.
        if !row_load_failed {
            ui.add_space(Spacing::MD);
            action |= if state.registry_has_tokens() {
                render_all_dismissed(ui, app_context, identity, state)
            } else {
                render_no_tokens(ui, app_context)
            };
        }
    } else {
        render_controls(ui, app_context, state, &rows);
        ui.add_space(Spacing::SM);
        let refreshing = state.refreshing(app_context);
        if refreshing {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(if state.first_load_shown(&rows, refreshing) {
                    FIRST_LOAD
                } else {
                    REFRESHING_TOOLTIP
                });
            });
            ui.add_space(Spacing::SM);
        }
        action |= render_list(ui, app_context, identity, state, &rows);
    }

    action |= render_stop_tracking_dialog(ui, state);
    if home.receive_open {
        action |= super::home::render_receive(ui, app_context, identity, home);
    }
    action
}

/// The arrival refresh, when due and nothing else claimed the frame.
pub fn load_action(app_context: &Arc<AppContext>, state: &mut IdentityTokensState) -> AppAction {
    state.load_action(app_context, crate::utils::time::now_ms())
}

fn render_header(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let synced = IdentityTokensState::can_read(app_context);
    ui.horizontal(|ui| {
        ui.label(RichText::new(HEADING).size(Typography::SCALE_XL).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action |= render_add_menu(ui, app_context);
            let refreshing = state.refreshing(app_context);
            let refresh = ui.add_enabled(
                synced && !refreshing,
                ComponentStyles::secondary_button(REFRESH_LABEL, dark),
            );
            if refresh
                .disabled_tooltip(refresh_block_reason(app_context))
                .clicked()
            {
                action |= state.refresh_action(app_context);
            }
            if refreshing {
                ui.add(egui::Spinner::new().size(Typography::SCALE_BASE));
            }
        });
    });
    action
}

/// Why a refresh control is disabled: a refresh already running, a network
/// still syncing, or no connection (also while one stops).
pub(super) fn refresh_block_reason(app_context: &AppContext) -> &'static str {
    let status = app_context.connection_status().spv_status();
    if IdentityTokensState::can_read(app_context) {
        REFRESHING_TOOLTIP
    } else if matches!(
        status,
        SpvStatus::Starting | SpvStatus::Syncing | SpvStatus::Running
    ) {
        DISABLED_NOT_SYNCED
    } else {
        DISABLED_NOT_CONNECTED
    }
}

/// "Add a token ▾": find by name in Tokens › Search, or add by ID.
fn render_add_menu(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;
    let (button, _) = MenuButton::from_button(egui::Button::new(ADD_TOKEN_LABEL).right_text("▾"))
        .ui(ui, |ui| {
            if ui.button(FIND_BY_NAME_LABEL).clicked() {
                action = AppAction::SetMainScreen(RootScreenType::RootScreenTokenSearch);
                ui.close();
            }
            if ui.button(ADD_BY_ID_LABEL).clicked() {
                action = AppAction::AddScreen(ScreenType::AddTokenById.create_screen(app_context));
                ui.close();
            }
        });
    // The glyph is decoration; the accessible name is the label alone.
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, ADD_TOKEN_LABEL));
    action
}

/// "Last updated …" and the receive link.
fn render_state_bar(ui: &mut Ui, state: &IdentityTokensState, home: &mut HomeState) {
    let dark = ui.visuals().dark_mode;
    ui.horizontal_wrapped(|ui| {
        if let Some(time_ms) = state.last_refreshed_at_ms() {
            let text = if state.refresh_failure().is_some() {
                stale_since_text(time_ms)
            } else {
                last_updated_text(time_ms)
            };
            ui.label(RichText::new(text).color(DashColors::text_secondary(dark)));
            ui.label(RichText::new("·").color(DashColors::text_secondary(dark)));
        }
        if ui.link(RECEIVE_LINK).clicked() {
            home.receive_open = true;
            home.receive_tokens_hint = true;
        }
    });
}

/// The inline error after a failed refresh; rows stay on screen.
fn render_failure(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
) -> AppAction {
    let Some(failure) = state.refresh_failure() else {
        return AppAction::None;
    };
    let synced = IdentityTokensState::can_read(app_context);
    let refreshing = state.refreshing(app_context);
    let text = failure.message().to_owned();
    ui.add_space(Spacing::SM);
    let response = InlineNotice::new(MessageType::Error, &text)
        .with_link_unless(
            TRY_AGAIN_LABEL,
            (!synced || refreshing).then_some(refresh_block_reason(app_context)),
        )
        .with_link(DISMISS_LABEL)
        .show(ui);
    ui.add_space(Spacing::SM);
    match response.clicked {
        Some(0) => state.refresh_action(app_context),
        Some(_) => {
            state.failure_dismissed = true;
            AppAction::None
        }
        None => AppAction::None,
    }
}

/// The inline error after the saved tokens could not be read, with a retry;
/// last good rows stay on screen. Returns whether it showed.
fn render_row_load_failure(ui: &mut Ui, state: &mut IdentityTokensState) -> bool {
    let Some(error) = state.row_load_error() else {
        return false;
    };
    let text = error.to_string();
    ui.add_space(Spacing::SM);
    let response = InlineNotice::new(MessageType::Error, &text)
        .with_link(TRY_AGAIN_LABEL)
        .show(ui);
    ui.add_space(Spacing::SM);
    if response.clicked.is_some() {
        state.invalidate_rows();
    }
    true
}

/// Hide-zero, saved as it changes, with how many rows it hides.
fn render_controls(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
    rows: &[TokenRow],
) {
    let mut prefs = state.prefs(app_context).clone();
    let before = prefs.clone();
    let dark = ui.visuals().dark_mode;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut prefs.hide_zero_balances, HIDE_ZERO_LABEL)
            .clickable_tooltip(HIDE_ZERO_TOOLTIP);
        let hidden = rows
            .iter()
            .filter(|row| !row_is_listed(row, &prefs))
            .count();
        if hidden > 0 {
            ui.label(
                RichText::new(hidden_count_text(hidden)).color(DashColors::text_secondary(dark)),
            );
        }
    });
    if prefs != before {
        state.set_prefs(app_context, prefs);
    }
}

fn render_list(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut IdentityTokensState,
    rows: &[TokenRow],
) -> AppAction {
    let prefs = state.prefs(app_context).clone();
    let mut listed: Vec<&TokenRow> = rows
        .iter()
        .filter(|row| row_is_listed(row, &prefs))
        .collect();
    sort_rows(&mut listed, prefs.sort);
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let context = RowContext::new(app_context, identity);
    // The list scrolls on its own so the heading and controls stay put on a
    // short window.
    egui::ScrollArea::vertical()
        .id_salt("identity_tokens_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            super::home::card(ui, |ui| {
                for (index, row) in listed.iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                    }
                    action |= render_row(ui, app_context, state, row, &context);
                }
            });
            ui.add_space(Spacing::SM);
            ui.label(
                RichText::new(FOOTNOTE)
                    .small()
                    .color(DashColors::text_secondary(dark)),
            );
        });
    action
}

fn render_row(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
    row: &TokenRow,
    context: &RowContext,
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let status = state.row_status(app_context, row, context.view_only);
    let wide = ui.available_width() >= WIDE_ROW_MIN_WIDTH;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = Spacing::SM;
        paint_token_monogram(ui, row, ROW_MONOGRAM_SIZE);
        let actions_width = if wide { ACTIONS_COLUMN_WIDTH } else { 0.0 };
        let balance_width = if wide { BALANCE_COLUMN_WIDTH } else { 0.0 };
        let name_width = (ui.available_width() - actions_width - balance_width - 2.0 * Spacing::SM)
            .max(NAME_COLUMN_MIN_WIDTH);
        ui.allocate_ui_with_layout(
            egui::vec2(name_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(name_width);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(row.name()).strong());
                    if row.issued_by_me {
                        accent_pill(ui, ISSUED_BY_YOU, DashColors::pill_accent(dark), None);
                    }
                    if !row.transferable() {
                        accent_pill(ui, CANT_BE_SENT_BADGE, DashColors::pill_neutral(dark), None);
                    }
                });
                ui.label(
                    RichText::new(shorten_id(&row.token_id_base58()))
                        .small()
                        .monospace()
                        .color(DashColors::text_secondary(dark)),
                );
                if !wide {
                    action |=
                        render_balance_and_status(ui, app_context, row, &status, context, state);
                }
            },
        );
        if wide {
            ui.allocate_ui_with_layout(
                egui::vec2(balance_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(balance_width);
                    action |=
                        render_balance_and_status(ui, app_context, row, &status, context, state);
                },
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action |= render_row_menu(ui, app_context, state, row, context);
        });
    });
    action
}

fn render_balance_and_status(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    row: &TokenRow,
    status: &RowStatus,
    context: &RowContext,
    state: &mut IdentityTokensState,
) -> AppAction {
    if let Some(balance) = row.balance() {
        ui.label(RichText::new(row.amount(balance).to_string()).monospace());
    }
    render_row_status(ui, app_context, row, status, context, state)
}

/// The row's one status line; "Try again" re-checks a failed pair once the
/// network is synced.
fn render_row_status(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    row: &TokenRow,
    status: &RowStatus,
    context: &RowContext,
    state: &mut IdentityTokensState,
) -> AppAction {
    let dark = ui.visuals().dark_mode;
    let secondary = DashColors::text_secondary(dark);
    let mut action = AppAction::None;
    match status {
        RowStatus::None => {}
        RowStatus::Checking => {
            ui.label(RichText::new(STATUS_CHECKING).small().color(secondary));
        }
        RowStatus::NoBalanceFound => {
            ui.label(
                RichText::new(STATUS_NO_BALANCE_FOUND)
                    .small()
                    .color(secondary),
            );
        }
        RowStatus::CouldNotCheck => {
            let refreshing = state.refreshing(app_context);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(STATUS_COULD_NOT_CHECK)
                        .small()
                        .color(DashColors::warning_color(dark)),
                );
                if ui
                    .add_enabled(
                        context.synced && !refreshing,
                        egui::Button::new(TRY_AGAIN_LABEL).small(),
                    )
                    .disabled_tooltip(context.block_reason)
                    .clicked()
                {
                    action = state.check_pair_action(app_context, row.pair());
                }
            });
        }
        RowStatus::AddKey => {
            ui.label(RichText::new(STATUS_ADD_KEY).small().color(secondary));
        }
        RowStatus::CanClaimOnce(amount) => {
            ui.label(
                RichText::new(can_claim_once_text(amount))
                    .small()
                    .color(DashColors::success_color(dark)),
            );
        }
        RowStatus::ClaimedOn(date) => {
            ui.label(
                RichText::new(claimed_on_text(date))
                    .small()
                    .color(secondary),
            );
        }
        RowStatus::RewardsMayBeWaiting => {
            ui.label(
                RichText::new(STATUS_REWARDS)
                    .small()
                    .color(DashColors::success_color(dark)),
            );
        }
    }
    if let Some(reward) = reward_beside_balance(row, status, context.view_only) {
        action |= render_row_status(ui, app_context, row, &reward, context, state);
    }
    action
}

/// The row's ••• menu.
fn render_row_menu(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
    row: &TokenRow,
    context: &RowContext,
) -> AppAction {
    let mut action = AppAction::None;
    let refreshing = state.refreshing(app_context);
    let response = ui.menu_button(MENU_LABEL, |ui| {
        if ui
            .add_enabled(
                context.synced && !refreshing,
                egui::Button::new(CHECK_NOW_LABEL),
            )
            .disabled_tooltip(context.block_reason)
            .clicked()
        {
            action |= state.check_pair_action(app_context, row.pair());
            ui.close();
        }
        if ui.button(COPY_ID_LABEL).clicked() {
            copy_with_confirmation(ui.ctx(), &row.token_id_base58(), COPIED_ID);
            ui.close();
        }
        if ui.button(STOP_TRACKING_LABEL).clicked() {
            open_stop_tracking_dialog(state, row, &context.identity_label);
            ui.close();
        }
    });
    response.response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, more_actions_label(row.name()))
    });
    action
}

/// Ask before a row leaves the list.
fn open_stop_tracking_dialog(
    state: &mut IdentityTokensState,
    row: &TokenRow,
    identity_label: &str,
) {
    state.stop_tracking_dialog = Some((
        ConfirmationDialog::new(
            STOP_TRACKING_TITLE,
            stop_tracking_question(row.name(), identity_label),
        )
        .confirm_text(Some(STOP_TRACKING_LABEL))
        .cancel_text(Some(CANCEL_LABEL)),
        row.pair(),
    ));
}

/// The Stop tracking confirmation, when open.
fn render_stop_tracking_dialog(ui: &mut Ui, state: &mut IdentityTokensState) -> AppAction {
    let Some((dialog, pair)) = &mut state.stop_tracking_dialog else {
        return AppAction::None;
    };
    let pair = *pair;
    match dialog.show(ui).inner.dialog_response {
        Some(ConfirmationStatus::Confirmed) => {
            state.stop_tracking_dialog = None;
            state.stop_tracking_action(pair)
        }
        Some(ConfirmationStatus::Canceled) => {
            state.stop_tracking_dialog = None;
            AppAction::None
        }
        None => AppAction::None,
    }
}

/// Nothing is tracked on this device yet.
fn render_no_tokens(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    super::home::card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(Spacing::MD);
            ui.label(
                RichText::new(EMPTY_HEADING)
                    .size(Typography::SCALE_LG)
                    .strong(),
            );
            ui.add_space(Spacing::XS);
            ui.label(RichText::new(EMPTY_BODY).color(DashColors::text_secondary(dark)));
            ui.add_space(Spacing::MD);
            ui.horizontal_wrapped(|ui| {
                if ComponentStyles::add_primary_button(ui, FIND_BY_NAME_LABEL).clicked() {
                    action = AppAction::SetMainScreen(RootScreenType::RootScreenTokenSearch);
                }
                if ComponentStyles::add_secondary_button(ui, ADD_BY_ID_LABEL, dark).clicked() {
                    action =
                        AppAction::AddScreen(ScreenType::AddTokenById.create_screen(app_context));
                }
            });
            ui.add_space(Spacing::MD);
        });
    });
    action
}

/// Tokens exist, but this identity stopped tracking all of them.
fn render_all_dismissed(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut IdentityTokensState,
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let identity_label = app_context.identity_display_label(identity);
    let synced = IdentityTokensState::can_read(app_context);
    let refreshing = state.refreshing(app_context);
    super::home::card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(Spacing::MD);
            ui.label(
                RichText::new(DISMISSED_HEADING)
                    .size(Typography::SCALE_LG)
                    .strong(),
            );
            ui.add_space(Spacing::XS);
            ui.label(
                RichText::new(dismissed_body_text(&identity_label))
                    .color(DashColors::text_secondary(dark)),
            );
            ui.add_space(Spacing::MD);
            ui.horizontal_wrapped(|ui| {
                let track_again = ui.add_enabled(
                    synced && !refreshing,
                    ComponentStyles::primary_button(TRACK_AGAIN_LABEL),
                );
                if track_again
                    .disabled_tooltip(refresh_block_reason(app_context))
                    .clicked()
                {
                    action = match state.track_again_action(app_context, identity) {
                        Ok(action) => action,
                        Err(error) => {
                            MessageBanner::set_global(ui.ctx(), &error, MessageType::Error)
                                .with_details(error);
                            AppAction::None
                        }
                    };
                }
                action |= render_add_menu(ui, app_context);
            });
            ui.add_space(Spacing::MD);
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disabled read control names what the user waits for: a refresh, a
    /// sync, or a connection (also while one stops).
    #[test]
    fn a_disabled_read_control_names_its_reason() {
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let status = context.connection_status();
        for (spv, masternodes_ready, reason) in [
            (SpvStatus::Running, true, REFRESHING_TOOLTIP),
            (SpvStatus::Syncing, false, DISABLED_NOT_SYNCED),
            (SpvStatus::Stopping, true, DISABLED_NOT_CONNECTED),
            (SpvStatus::Stopped, false, DISABLED_NOT_CONNECTED),
        ] {
            status.set_spv_status(spv);
            status.set_masternodes_ready(masternodes_ready);
            assert_eq!(refresh_block_reason(&context), reason, "{spv:?}");
        }
    }
}
