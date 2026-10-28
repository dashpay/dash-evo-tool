//! Identity Hub Tokens tab: the tokens the selected identity holds, with Send
//! and Claim. Holder actions only — every administrative action stays in
//! Tokens › My Tokens; the only link there is "Manage in Tokens screen", shown
//! when the identity actually holds an admin permission on the token.
//!
//! Design: `docs/ai-design/2026-10-28-identity-tokens/` (frames T1–T4, T7, T9).

use super::home::HomeState;
use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::datetime;
use crate::model::identity_name::shorten_id;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::components::pill::accent_pill;
use crate::ui::identity::identity_hero_card::HeroIdentityKind;
use crate::ui::state::identity_tokens::{
    IdentityTokensState, MANY_TOKENS_THRESHOLD, RefreshFailure, RowStatus, TokenRow, row_is_listed,
    row_status, sort_rows,
};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape, Spacing};
use crate::ui::tokens::claim_tokens_screen::ClaimTokensScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenType};
use egui::{self, RichText, Ui};
use std::sync::Arc;

pub const HEADING: &str = "Tokens";
pub const SUBTITLE: &str = "Tokens this identity holds or keeps an eye on.";
pub const REFRESH_LABEL: &str = "Refresh";
pub const REFRESHING_TOOLTIP: &str = "Balances are being checked.";
pub const ADD_TOKEN_LABEL: &str = "Add a token";
pub const FIND_BY_NAME_LABEL: &str = "Find a token by name";
pub const ADD_BY_ID_LABEL: &str = "Add by token or contract ID";
pub const RECEIVE_LINK: &str = "How do I receive tokens?";
pub const HIDE_ZERO_LABEL: &str = "Hide tokens with zero balance";
pub const HIDE_ZERO_TOOLTIP: &str =
    "Hides tokens whose balance is 0. Tokens still being checked always stay visible.";
pub const FILTER_HINT: &str = "Filter by name or ID";
pub const SORT_LABEL: &str = "Sort";
pub const SEND_LABEL: &str = "Send";
pub const CLAIM_LABEL: &str = "Claim";
pub const MENU_LABEL: &str = "•••";
pub const CHECK_NOW_LABEL: &str = "Check balance now";
pub const DETAILS_LABEL: &str = "Token details";
pub const COPY_ID_LABEL: &str = "Copy token ID";
pub const OPEN_DETAILS_TOOLTIP: &str = "Show this token's details.";
pub const STOP_TRACKING_LABEL: &str = "Stop tracking";
pub const STOP_TRACKING_TITLE: &str = "Stop tracking this token?";
pub const MANAGE_LABEL: &str = "Manage in Tokens screen";
pub const TRY_AGAIN_LABEL: &str = "Try again";
pub const DISMISS_LABEL: &str = "Dismiss";
pub const TRACK_AGAIN_LABEL: &str = "Track tokens again";
pub const ISSUED_BY_YOU_BADGE: &str = "Issued by you";
pub const CANT_BE_SENT_BADGE: &str = "Can't be sent";
pub const STATUS_CHECKING: &str = "Checking balance…";
pub const STATUS_COULD_NOT_CHECK: &str = "Couldn't check this balance.";
pub const STATUS_REWARDS: &str = "Rewards may be waiting for you.";
pub const STATUS_ADD_KEY: &str = "Add a key to this identity to send or claim tokens.";
pub const DISABLED_NOT_TRANSFERABLE: &str = "This token cannot be sent to other identities.";
pub const DISABLED_NOT_SYNCED: &str = "Available after sync finishes.";
pub const EMPTY_HEADING: &str = "No tokens yet";
pub const EMPTY_BODY: &str = "Tokens are assets issued on Dash Platform. A token shows here once you add it, so when someone sends you one, add it to see your balance.";
pub const DISMISSED_HEADING: &str = "Nothing tracked for this identity";
pub const FIRST_LOAD: &str =
    "Checking balances for the first time. This usually takes a few seconds.";
pub const FOOTNOTE: &str = "Balances are checked in the background. A token you don't see here can be added with Add a token.";
pub const ERROR_REFRESH_FAILED: &str =
    "Token balances couldn't be updated. Check your internet connection and try again.";
pub const ERROR_REFRESH_SLOW: &str =
    "Updating token balances is taking longer than usual. Try again in a moment.";
pub const ERROR_CONTRACT_MISSING: &str =
    "This token's details aren't available yet. Refresh and try again.";
pub const ERROR_CONTRACT_UNREADABLE: &str =
    "The token contract could not be loaded. Refresh and try again.";

/// Every complete sentence of the tab, for the copy test.
#[cfg(test)]
const SENTENCES: &[&str] = &[
    SUBTITLE,
    REFRESHING_TOOLTIP,
    HIDE_ZERO_TOOLTIP,
    OPEN_DETAILS_TOOLTIP,
    STATUS_COULD_NOT_CHECK,
    STATUS_REWARDS,
    STATUS_ADD_KEY,
    DISABLED_NOT_TRANSFERABLE,
    DISABLED_NOT_SYNCED,
    EMPTY_BODY,
    FIRST_LOAD,
    FOOTNOTE,
    ERROR_REFRESH_FAILED,
    ERROR_REFRESH_SLOW,
    ERROR_CONTRACT_MISSING,
    ERROR_CONTRACT_UNREADABLE,
];

/// "Last updated {time}."
pub fn last_updated_text(time_ms: u64) -> String {
    format!(
        "Last updated {}.",
        datetime::local_date_time_from_unix_millis(time_ms)
    )
}

/// "Showing balances from {time}." — after a failed refresh.
pub fn stale_since_text(time_ms: u64) -> String {
    format!(
        "Showing balances from {}.",
        datetime::local_date_time_from_unix_millis(time_ms)
    )
}

/// "You can claim {amount} once."
pub fn can_claim_once_text(amount: &crate::model::amount::Amount) -> String {
    format!("You can claim {amount} once.")
}

/// "Claimed on {date}."
pub fn claimed_on_text(date: &str) -> String {
    format!("Claimed on {date}.")
}

/// "{count} hidden"
pub fn hidden_count_text(count: usize) -> String {
    format!("{count} hidden")
}

/// "You stopped tracking every token for {identity}. …"
pub fn dismissed_body_text(identity: &str) -> String {
    format!(
        "You stopped tracking every token for {identity}. Tokens you track on other identities are unaffected."
    )
}

/// "Stop tracking {token} for {identity}? …"
pub fn stop_tracking_question(token: &str, identity: &str) -> String {
    format!("Stop tracking {token} for {identity}? You can add it again at any time.")
}

/// Tooltip of the tab badge: "{count} tokens have something to claim."
pub fn pending_claims_text(count: usize) -> String {
    format!("{count} tokens have something to claim.")
}

/// Whether the identity can sign nothing on this device.
fn view_only(identity: &QualifiedIdentity) -> bool {
    identity.private_keys.is_empty()
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
    let rows: Vec<TokenRow> = state.rows(app_context, identity).to_vec();

    if let Some(pair) = state.detail {
        // The row can leave while its page is open (stopped tracking elsewhere).
        action |= match rows.iter().find(|row| row.pair == pair) {
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
        action |= render_controls(ui, app_context, state, &rows);
        ui.add_space(Spacing::SM);
        if rows.iter().all(|row| row.balance.is_none()) && state.last_refreshed_at_ms().is_none() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label(FIRST_LOAD);
            });
            ui.add_space(Spacing::SM);
        }
        action |= render_list(ui, app_context, identity, state, rows);
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
    ui.horizontal(|ui| {
        ui.label(RichText::new(HEADING).size(20.0).strong());
        if rows.len() >= MANY_TOKENS_THRESHOLD {
            ui.label(
                RichText::new(format!("{} tokens", rows.len()))
                    .color(DashColors::text_secondary(dark)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action |= render_add_menu(ui, app_context);
            let refreshing =
                state.refresh_pending() || app_context.token_balance_refresh_in_flight();
            let refresh = ui.add_enabled(
                !refreshing,
                ComponentStyles::secondary_button(REFRESH_LABEL, dark),
            );
            if refresh.disabled_tooltip(REFRESHING_TOOLTIP).clicked() {
                action |= state.refresh_action(app_context);
            }
            if refreshing {
                ui.add(egui::Spinner::new().size(16.0));
            }
        });
    });
    action
}

/// "Add a token ▾": find by name in Tokens › Search, or add by ID.
fn render_add_menu(ui: &mut Ui, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;
    ui.menu_button(format!("{ADD_TOKEN_LABEL} ▾"), |ui| {
        if ui.button(FIND_BY_NAME_LABEL).clicked() {
            action = AppAction::SetMainScreen(RootScreenType::RootScreenTokenSearch);
            ui.close();
        }
        if ui.button(ADD_BY_ID_LABEL).clicked() {
            action = AppAction::AddScreen(ScreenType::AddTokenById.create_screen(app_context));
            ui.close();
        }
    });
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
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    ui.add_space(Spacing::SM);
    egui::Frame::new()
        .fill(DashColors::message_background_color(
            MessageType::Error,
            dark,
        ))
        .corner_radius(Shape::RADIUS_MD)
        .inner_margin(egui::Margin::same(Spacing::SM_I8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("▲").color(DashColors::message_color(MessageType::Error, dark)),
                );
                ui.label(match failure {
                    RefreshFailure::TooSlow => ERROR_REFRESH_SLOW,
                    RefreshFailure::Failed => ERROR_REFRESH_FAILED,
                });
                if ui.link(TRY_AGAIN_LABEL).clicked() {
                    action |= state.refresh_action(app_context);
                }
                if ui.link(DISMISS_LABEL).clicked() {
                    state.failure_dismissed = true;
                }
            });
        });
    ui.add_space(Spacing::SM);
    action
}

/// Hide-zero (always), plus filter and sort once the list is long.
fn render_controls(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
    rows: &[TokenRow],
) -> AppAction {
    let mut prefs = state.prefs(app_context).clone();
    let before = prefs.clone();
    let dark = ui.visuals().dark_mode;
    ui.horizontal_wrapped(|ui| {
        if rows.len() >= MANY_TOKENS_THRESHOLD {
            ui.add(
                egui::TextEdit::singleline(&mut prefs.filter)
                    .hint_text(FILTER_HINT)
                    .desired_width(200.0),
            );
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
    AppAction::None
}

fn render_list(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut IdentityTokensState,
    mut rows: Vec<TokenRow>,
) -> AppAction {
    let prefs = state.prefs(app_context).clone();
    sort_rows(&mut rows, prefs.sort);
    let listed: Vec<TokenRow> = rows
        .into_iter()
        .filter(|row| row_is_listed(row, &prefs))
        .collect();
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let synced = app_context.connection_status().masternodes_ready();
    let view_only = view_only(identity);
    let identity_label = app_context.identity_display_label(identity);
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
                    action |= render_row(
                        ui,
                        app_context,
                        state,
                        row,
                        RowContext {
                            synced,
                            view_only,
                            identity_label: &identity_label,
                        },
                    );
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

/// What every row needs to know about the identity and the network.
#[derive(Clone, Copy)]
pub(super) struct RowContext<'a> {
    pub(super) synced: bool,
    pub(super) view_only: bool,
    pub(super) identity_label: &'a str,
}

fn render_row(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut IdentityTokensState,
    row: &TokenRow,
    context: RowContext<'_>,
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let status = row_status(row, context.view_only);
    let wide = ui.available_width() >= 640.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = Spacing::SM;
        super::avatar::paint_identity_monogram(
            ui,
            36.0,
            HeroIdentityKind::User,
            row.name.chars().next(),
            DashColors::pastel_gradient(row.pair.token_id.as_bytes()[0] as usize),
        );
        let actions_width = if wide { 220.0 } else { 0.0 };
        let balance_width = if wide { 200.0 } else { 0.0 };
        let name_width =
            (ui.available_width() - actions_width - balance_width - 2.0 * Spacing::SM).max(120.0);
        ui.allocate_ui_with_layout(
            egui::vec2(name_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(name_width);
                ui.horizontal_wrapped(|ui| {
                    // The name opens the detail page; Enter works once focused.
                    if ui
                        .add(egui::Button::new(RichText::new(&row.name).strong()).frame(false))
                        .clickable_tooltip(OPEN_DETAILS_TOOLTIP)
                        .clicked()
                    {
                        state.detail = Some(row.pair);
                    }
                    if row.issued_by_me {
                        accent_pill(ui, ISSUED_BY_YOU_BADGE, DashColors::DASH_BLUE, None);
                    }
                    if !row.transferable {
                        accent_pill(
                            ui,
                            CANT_BE_SENT_BADGE,
                            DashColors::text_secondary(dark),
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
                    render_balance_and_status(ui, row, &status, &mut action, state);
                }
            },
        );
        if wide {
            ui.allocate_ui_with_layout(
                egui::vec2(balance_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(balance_width);
                    render_balance_and_status(ui, row, &status, &mut action, state);
                },
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            action |= render_row_menu(ui, state, row, context.identity_label);
            action |= render_row_actions(ui, app_context, row, context);
        });
    });
    action
}

fn render_balance_and_status(
    ui: &mut Ui,
    row: &TokenRow,
    status: &RowStatus,
    action: &mut AppAction,
    state: &mut IdentityTokensState,
) {
    if let Some(balance) = row.balance {
        ui.label(RichText::new(row.amount(balance).to_string()).monospace());
    }
    render_row_status(ui, row, status, action, state);
}

/// The row's one status line; "Try again" re-checks a failed pair.
pub(super) fn render_row_status(
    ui: &mut Ui,
    row: &TokenRow,
    status: &RowStatus,
    action: &mut AppAction,
    state: &mut IdentityTokensState,
) {
    let dark = ui.visuals().dark_mode;
    let secondary = DashColors::text_secondary(dark);
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
                if ui.small_button(TRY_AGAIN_LABEL).clicked() {
                    *action |= state.check_pair_action(row.pair);
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
}

/// Send and Claim, laid out right to left. Both wait for the masternode list
/// and for a signing key; Send also needs a balance and a transferable token.
pub(super) fn render_row_actions(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    row: &TokenRow,
    context: RowContext<'_>,
) -> AppAction {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    let blocked_reason = if !context.synced {
        Some(DISABLED_NOT_SYNCED)
    } else if context.view_only {
        Some(STATUS_ADD_KEY)
    } else {
        None
    };
    if row.claim_available() {
        let claim = ui.add_enabled(
            blocked_reason.is_none(),
            ComponentStyles::secondary_button(CLAIM_LABEL, dark),
        );
        if claim
            .disabled_tooltip(blocked_reason.unwrap_or_default())
            .clicked()
        {
            action |= claim_action(ui.ctx(), app_context, row);
        }
    }
    if row.holds_some() {
        let send_reason = if row.transferable {
            blocked_reason
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

/// Push the Claim screen for `row`, or explain why not.
pub(super) fn claim_action(
    ctx: &egui::Context,
    app_context: &Arc<AppContext>,
    row: &TokenRow,
) -> AppAction {
    match app_context.get_contract_by_token_id(&row.pair.token_id) {
        Ok(Some(contract)) => {
            AppAction::AddScreen(Screen::ClaimTokensScreen(ClaimTokensScreen::new(
                row.basic_info(),
                contract,
                row.token_config.clone(),
                app_context,
            )))
        }
        Ok(None) => {
            MessageBanner::set_global(ctx, ERROR_CONTRACT_MISSING, MessageType::Info);
            AppAction::None
        }
        Err(error) => {
            MessageBanner::set_global(ctx, ERROR_CONTRACT_UNREADABLE, MessageType::Error)
                .with_details(error);
            AppAction::None
        }
    }
}

/// The row's ••• menu.
fn render_row_menu(
    ui: &mut Ui,
    state: &mut IdentityTokensState,
    row: &TokenRow,
    identity_label: &str,
) -> AppAction {
    let mut action = AppAction::None;
    let response = ui.menu_button(MENU_LABEL, |ui| {
        if ui.button(CHECK_NOW_LABEL).clicked() {
            action |= state.check_pair_action(row.pair);
            ui.close();
        }
        if ui.button(DETAILS_LABEL).clicked() {
            state.detail = Some(row.pair);
            ui.close();
        }
        if ui.button(COPY_ID_LABEL).clicked() {
            ui.ctx().copy_text(row.token_id_base58());
            ui.close();
        }
        if ui.button(STOP_TRACKING_LABEL).clicked() {
            open_stop_tracking_dialog(state, row, identity_label);
            ui.close();
        }
        if row.has_admin_permission {
            ui.separator();
            if ui.button(MANAGE_LABEL).clicked() {
                action |= AppAction::SetMainScreen(RootScreenType::RootScreenMyTokenBalances);
                ui.close();
            }
        }
    });
    response.response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("More actions for {}", row.name),
        )
    });
    action
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
            stop_tracking_question(&row.name, identity_label),
        )
        .confirm_text(Some(STOP_TRACKING_LABEL))
        .cancel_text(Some("Cancel")),
        row.pair,
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
            ui.label(RichText::new(EMPTY_HEADING).size(18.0).strong());
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
    super::home::card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(Spacing::MD);
            ui.label(RichText::new(DISMISSED_HEADING).size(18.0).strong());
            ui.add_space(Spacing::XS);
            ui.label(
                RichText::new(dismissed_body_text(&identity_label))
                    .color(DashColors::text_secondary(dark)),
            );
            ui.add_space(Spacing::MD);
            ui.horizontal_wrapped(|ui| {
                if ComponentStyles::add_primary_button(ui, TRACK_AGAIN_LABEL).clicked() {
                    action = state.track_again_action(app_context, identity);
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

    #[test]
    fn every_sentence_is_complete() {
        for sentence in SENTENCES {
            assert!(
                sentence.ends_with('.'),
                "'{sentence}' must be a complete sentence"
            );
        }
        assert!(STATUS_CHECKING.ends_with('…'));
        assert!(
            last_updated_text(0).starts_with("Last updated ")
                && last_updated_text(0).ends_with('.')
        );
        assert!(stale_since_text(0).ends_with('.'));
        assert!(claimed_on_text("2026-10-20").ends_with('.'));
        assert!(stop_tracking_question("DCF", "Alice").ends_with('.'));
        assert!(dismissed_body_text("Alice").ends_with('.'));
        assert!(pending_claims_text(2).ends_with('.'));
        assert_eq!(hidden_count_text(3), "3 hidden");
    }

    #[test]
    fn no_hub_copy_uses_token_jargon() {
        for text in SENTENCES.iter().copied().chain([
            HEADING,
            EMPTY_HEADING,
            DISMISSED_HEADING,
            CHECK_NOW_LABEL,
            STOP_TRACKING_LABEL,
            MANAGE_LABEL,
            SEND_LABEL,
        ]) {
            let lower = text.to_lowercase();
            for banned in [
                "transfer",
                "perpetual",
                "pre-programmed",
                "contract owner",
                "state transition",
                "nonce",
            ] {
                assert!(!lower.contains(banned), "'{text}' uses '{banned}'");
            }
        }
    }
}
