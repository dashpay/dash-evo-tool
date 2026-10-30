//! Identity Hub Tokens tab: the tokens the selected identity holds, with Send
//! and Claim. Holder actions only — every administrative action stays in
//! Tokens › My Tokens; the only link there is "Manage in Tokens screen", shown
//! when the identity actually holds an admin permission on the token.
//!
//! Copy lives in `token_copy.rs`; state in `ui/state/identity_tokens.rs`.
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` (frames T1–T4, T7, T9).

use super::home::HomeState;
use super::token_copy::*;
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::identity_name::shorten_id;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::IdentityTokensViewPrefs;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::inline_notice::InlineNotice;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::components::pill::accent_pill;
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::state::identity_tokens::{
    IdentityTokensState, RefreshFailure, RowStatus, TokenRow, filter_shown, row_is_listed,
    row_status, sort_rows,
};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Spacing, Typography};
use crate::ui::tokens::claim_tokens_screen::ClaimTokensScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenType};
use egui::containers::menu::MenuButton;
use egui::{self, RichText, Sense, Ui, UiBuilder};
use std::sync::Arc;

/// A wide row lays the balance and the actions out as columns beside the
/// name; a narrow one stacks them under it.
const WIDE_ROW_MIN_WIDTH: f32 = 640.0;
const ACTIONS_COLUMN_WIDTH: f32 = 220.0;
const BALANCE_COLUMN_WIDTH: f32 = 200.0;
const NAME_COLUMN_MIN_WIDTH: f32 = 120.0;
const FILTER_WIDTH: f32 = 200.0;
/// The token monogram on a row and on the detail page.
pub(super) const ROW_MONOGRAM_SIZE: f32 = 36.0;
pub(super) const DETAIL_MONOGRAM_SIZE: f32 = 56.0;

/// What every row needs to know about the identity and the network.
pub(super) struct RowContext {
    pub(super) synced: bool,
    /// The identity can sign nothing on this device.
    pub(super) view_only: bool,
    pub(super) identity_label: String,
}

impl RowContext {
    pub(super) fn new(app_context: &AppContext, identity: &QualifiedIdentity) -> Self {
        Self {
            synced: app_context.connection_status().masternodes_ready(),
            view_only: identity.private_keys.is_empty(),
            identity_label: app_context.identity_display_label(identity),
        }
    }

    /// Why Send and Claim are blocked for this identity right now, if they are.
    fn holder_action_block(&self) -> Option<&'static str> {
        if !self.synced {
            Some(DISABLED_NOT_SYNCED)
        } else if self.view_only {
            Some(STATUS_ADD_KEY)
        } else {
            None
        }
    }
}

/// The token's monogram: its initial on a hue derived from its ID.
pub(super) fn paint_token_monogram(ui: &mut Ui, row: &TokenRow, size: f32) {
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

    if let Some(pair) = state.detail {
        // The row can leave while its page is open (stopped tracking elsewhere).
        action |= match rows.iter().find(|row| row.pair() == pair) {
            Some(row) => super::token_detail::render(ui, app_context, identity, row, state),
            None => {
                state.detail = None;
                AppAction::None
            }
        };
        action |= render_stop_tracking_dialog(ui, state);
        return action;
    }

    action |= render_header(ui, app_context, state, &rows);
    ui.add_space(Spacing::XS);
    ui.label(
        RichText::new(SUBTITLE)
            .small()
            .color(DashColors::text_secondary(ui.visuals().dark_mode)),
    );
    ui.add_space(Spacing::SM);
    render_state_bar(ui, state, home);
    action |= render_failure(ui, app_context, state);

    if rows.is_empty() {
        ui.add_space(Spacing::MD);
        action |= if state.registry_has_tokens() {
            render_all_dismissed(ui, app_context, identity, state)
        } else {
            render_no_tokens(ui, app_context)
        };
    } else {
        render_controls(ui, app_context, state, &rows);
        ui.add_space(Spacing::SM);
        if state.first_load_shown(&rows, state.refreshing(app_context)) {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(FIRST_LOAD);
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
    rows: &[TokenRow],
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let synced = app_context.connection_status().masternodes_ready();
    ui.horizontal(|ui| {
        ui.label(RichText::new(HEADING).size(Typography::SCALE_XL).strong());
        if filter_shown(rows.len()) {
            ui.label(
                RichText::new(tracked_count_text(rows.len()))
                    .color(DashColors::text_secondary(dark)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action |= render_add_menu(ui, app_context);
            let refreshing = state.refreshing(app_context);
            let refresh = ui.add_enabled(
                synced && !refreshing,
                ComponentStyles::secondary_button(REFRESH_LABEL, dark),
            );
            if refresh
                .disabled_tooltip(refresh_block_reason(synced))
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

/// Why a refresh control is disabled: the network, or a refresh already running.
fn refresh_block_reason(synced: bool) -> &'static str {
    if synced {
        REFRESHING_TOOLTIP
    } else {
        DISABLED_NOT_SYNCED
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
    button
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, ADD_TOKEN_LABEL));
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
pub(super) fn render_failure(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
) -> AppAction {
    let Some(failure) = state.refresh_failure() else {
        return AppAction::None;
    };
    let synced = app_context.connection_status().masternodes_ready();
    let text = match failure {
        RefreshFailure::TooSlow => ERROR_REFRESH_SLOW,
        RefreshFailure::Failed => ERROR_REFRESH_FAILED,
    };
    ui.add_space(Spacing::SM);
    let response = InlineNotice::new(MessageType::Error, text)
        .with_link_unless(TRY_AGAIN_LABEL, (!synced).then_some(DISABLED_NOT_SYNCED))
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

/// Hide-zero (always), plus filter and sort once the list is long. Hide-zero
/// and sort are saved as they change; the filter applies at once but is
/// written when its box loses focus (Enter included), when the tab is left
/// or when the identity changes — never per keystroke.
fn render_controls(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
    rows: &[TokenRow],
) {
    let mut prefs = state.prefs(app_context).clone();
    let before = prefs.clone();
    let dark = ui.visuals().dark_mode;
    let filter_shown = filter_shown(rows.len());
    let mut filter_done = false;
    ui.horizontal_wrapped(|ui| {
        if filter_shown {
            let filter = ui.add(
                egui::TextEdit::singleline(&mut prefs.filter)
                    .hint_text(FILTER_HINT)
                    .char_limit(IdentityTokensViewPrefs::MAX_FILTER_CHARS)
                    .desired_width(FILTER_WIDTH),
            );
            filter_done = filter.lost_focus();
            egui::ComboBox::from_id_salt("identity_tokens_sort")
                .selected_text(prefs.sort.label())
                .show_ui(ui, |ui| {
                    for sort in crate::model::token::TokenListSort::ALL {
                        ui.selectable_value(&mut prefs.sort, sort, sort.label());
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, SORT_LABEL)
                });
        }
        ui.checkbox(&mut prefs.hide_zero_balances, HIDE_ZERO_LABEL)
            .clickable_tooltip(HIDE_ZERO_TOOLTIP);
        let hidden = rows
            .iter()
            .filter(|row| !row_is_listed(row, &prefs, filter_shown))
            .count();
        if hidden > 0 {
            ui.label(
                RichText::new(hidden_count_text(hidden)).color(DashColors::text_secondary(dark)),
            );
        }
    });
    let a_control_changed =
        prefs.hide_zero_balances != before.hide_zero_balances || prefs.sort != before.sort;
    if prefs != before {
        state.edit_prefs(prefs);
    }
    if filter_done || a_control_changed {
        state.save_prefs(app_context);
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
    let filter_shown = filter_shown(rows.len());
    let mut listed: Vec<&TokenRow> = rows
        .iter()
        .filter(|row| row_is_listed(row, &prefs, filter_shown))
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
    let status = row_status(row, context.view_only);
    let wide = ui.available_width() >= WIDE_ROW_MIN_WIDTH;
    // The whole row opens the detail page; its own buttons and menu keep
    // their clicks, while plain text lets the click through.
    let row_response = ui.scope_builder(UiBuilder::new().sense(Sense::click()), |ui| {
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = Spacing::SM;
            paint_token_monogram(ui, row, ROW_MONOGRAM_SIZE);
            let actions_width = if wide { ACTIONS_COLUMN_WIDTH } else { 0.0 };
            let balance_width = if wide { BALANCE_COLUMN_WIDTH } else { 0.0 };
            let name_width =
                (ui.available_width() - actions_width - balance_width - 2.0 * Spacing::SM)
                    .max(NAME_COLUMN_MIN_WIDTH);
            ui.allocate_ui_with_layout(
                egui::vec2(name_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(name_width);
                    ui.horizontal_wrapped(|ui| {
                        // The name opens the detail page; Enter works once focused.
                        if ui
                            .add(egui::Button::new(RichText::new(row.name()).strong()).frame(false))
                            .clickable_tooltip(OPEN_DETAILS_TOOLTIP)
                            .clicked()
                        {
                            state.detail = Some(row.pair());
                        }
                        if row.issued_by_me {
                            accent_pill(
                                ui,
                                ISSUED_BY_YOU_BADGE,
                                DashColors::pill_accent(dark),
                                None,
                            );
                        }
                        if !row.transferable() {
                            accent_pill(
                                ui,
                                CANT_BE_SENT_BADGE,
                                DashColors::pill_neutral(dark),
                                None,
                            );
                        }
                    });
                    ui.label(
                        RichText::new(shorten_id(&row.token_id_base58()))
                            .small()
                            .monospace()
                            .color(DashColors::text_secondary(dark)),
                    );
                    if !wide {
                        action |= render_balance_and_status(
                            ui,
                            app_context,
                            row,
                            &status,
                            context,
                            state,
                        );
                    }
                },
            );
            if wide {
                ui.allocate_ui_with_layout(
                    egui::vec2(balance_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(balance_width);
                        action |= render_balance_and_status(
                            ui,
                            app_context,
                            row,
                            &status,
                            context,
                            state,
                        );
                    },
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                action |= render_row_menu(ui, app_context, state, row, context);
                action |= render_row_actions(ui, app_context, row, context);
            });
        });
    });
    if row_response.response.clicked() {
        state.detail = Some(row.pair());
    }
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
pub(super) fn render_row_status(
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
        RowStatus::CouldNotCheck => {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(STATUS_COULD_NOT_CHECK)
                        .small()
                        .color(DashColors::warning_color(dark)),
                );
                if ui
                    .add_enabled(context.synced, egui::Button::new(TRY_AGAIN_LABEL).small())
                    .disabled_tooltip(DISABLED_NOT_SYNCED)
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
    action
}

/// Send and Claim, laid out right to left. Both wait for the masternode list
/// and for a signing key; Send also needs a balance and a transferable token.
pub(super) fn render_row_actions(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    row: &TokenRow,
    context: &RowContext,
) -> AppAction {
    let mut action = render_claim_button(ui, app_context, row, context);
    if row.holds_some() {
        let send_reason = if row.transferable() {
            context.holder_action_block()
        } else {
            Some(DISABLED_NOT_TRANSFERABLE)
        };
        let send = ui.add_enabled(
            send_reason.is_none(),
            ComponentStyles::primary_button(SEND_LABEL),
        );
        if send
            .disabled_tooltip(send_reason.unwrap_or_default())
            .clicked()
            && let Some(balance) = row.balance_record()
        {
            action |= AppAction::AddScreen(
                ScreenType::TransferTokensScreen(balance).create_screen(app_context),
            );
        }
    }
    action
}

/// The Claim button, when `row` has something to claim; disabled with its
/// reason until the network is synced and the identity can sign.
pub(super) fn render_claim_button(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    row: &TokenRow,
    context: &RowContext,
) -> AppAction {
    if !row.claim_available() {
        return AppAction::None;
    }
    let blocked_reason = context.holder_action_block();
    let claim = ui.add_enabled(
        blocked_reason.is_none(),
        ComponentStyles::secondary_button(CLAIM_LABEL, ui.visuals().dark_mode),
    );
    if claim
        .disabled_tooltip(blocked_reason.unwrap_or_default())
        .clicked()
    {
        return claim_action(ui.ctx(), app_context, row);
    }
    AppAction::None
}

/// Push the Claim screen for `row`, or explain why not.
fn claim_action(ctx: &egui::Context, app_context: &Arc<AppContext>, row: &TokenRow) -> AppAction {
    match app_context.get_contract_by_token_id(&row.token.token_id) {
        Ok(Some(contract)) => {
            AppAction::AddScreen(Screen::ClaimTokensScreen(ClaimTokensScreen::new(
                row.basic_info(),
                contract,
                row.token.token_config.clone(),
                app_context,
            )))
        }
        Ok(None) => {
            MessageBanner::set_global(ctx, ERROR_CONTRACT_MISSING, MessageType::Info);
            AppAction::None
        }
        Err(error) => {
            MessageBanner::set_global(ctx, ERROR_CONTRACT_MISSING, MessageType::Error)
                .with_details(error);
            AppAction::None
        }
    }
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
    let response = ui.menu_button(MENU_LABEL, |ui| {
        if ui
            .add_enabled(context.synced, egui::Button::new(CHECK_NOW_LABEL))
            .disabled_tooltip(DISABLED_NOT_SYNCED)
            .clicked()
        {
            action |= state.check_pair_action(app_context, row.pair());
            ui.close();
        }
        if ui.button(DETAILS_LABEL).clicked() {
            state.detail = Some(row.pair());
            ui.close();
        }
        if ui.button(COPY_ID_LABEL).clicked() {
            ui.ctx().copy_text(row.token_id_base58());
            ui.close();
        }
        if ui.button(STOP_TRACKING_LABEL).clicked() {
            open_stop_tracking_dialog(state, row, &context.identity_label);
            ui.close();
        }
        if row.has_admin_permission {
            ui.separator();
            if ui.button(MANAGE_LABEL).clicked() {
                action |= manage_action(app_context, row);
                ui.close();
            }
        }
    });
    response.response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            more_actions_label(row.name()),
        )
    });
    action
}

/// Open Tokens › My Tokens on this token, where the administrative actions
/// live.
pub(super) fn manage_action(app_context: &Arc<AppContext>, row: &TokenRow) -> AppAction {
    app_context.set_pending_token_selection(row.token.token_id);
    AppAction::SetMainScreen(RootScreenType::RootScreenMyTokenBalances)
}

/// Ask before a row leaves the list.
pub(super) fn open_stop_tracking_dialog(
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
pub(super) fn render_stop_tracking_dialog(
    ui: &mut Ui,
    state: &mut IdentityTokensState,
) -> AppAction {
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
    let synced = app_context.connection_status().masternodes_ready();
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
                    .disabled_tooltip(refresh_block_reason(synced))
                    .clicked()
                {
                    action = state.track_again_action(app_context, identity);
                }
                action |= render_add_menu(ui, app_context);
            });
            ui.add_space(Spacing::MD);
        });
    });
    action
}
