//! Identity summary and Home: shared send flows, setup, payments, and contacts.

use super::onboarding_checklist::{ChecklistAction, ChecklistStep, OnboardingChecklist};
use super::profile_cache::ProfileCache;
use super::usernames_card::{UsernameRow, register_action_for, username_rows};
use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::fee_estimation::{format_credits_as_dash, format_duffs_as_dash};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::ScreenType;
use crate::ui::components::avatar::Avatar;
use crate::ui::identity::tabs::IdentityHubTab;
pub use crate::ui::state::identity_home::HomeState;
use crate::ui::state::{AvatarCache, contacts_view::ContactsState};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt, Shape, Spacing};
use crate::ui::wallets::send_screen::IdentitySendMode;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{self, Frame, Margin, RichText, Ui};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeOutcome {
    None,
    GoToContacts,
    GoToSettings,
    GoToUsernames,
    GoToAdvanced,
    DismissChecklist,
}

/// Render the selected identity above every hub tab.
pub fn render_header(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    profiles: &mut ProfileCache,
    avatars: &mut AvatarCache,
    pending_avatars: &mut Vec<BackendTask>,
) -> (AppAction, HomeOutcome) {
    let Some(identity) = app_context.resolve_selected_identity() else {
        return (AppAction::None, HomeOutcome::None);
    };
    profiles.seed_cached(app_context, &identity);
    let profile = profiles.get_or_request(&identity).and_then(Option::as_ref);
    let name = app_context.identity_display_name_or(
        identity.identity.id(),
        profile.and_then(|p| p.display_name_opt()),
    );
    let heading = name.unwrap_or_else(|| app_context.identity_display_label(&identity));
    let avatar_url = profile
        .map(|p| p.avatar_url.as_str())
        .filter(|url| !url.is_empty());
    let main = app_context.main_username(&identity);
    let requests = app_context.username_requests_for(&identity.identity.id());
    let names: Vec<_> = username_rows(&identity, main.as_deref(), &requests)
        .into_iter()
        .filter_map(|row| match row {
            UsernameRow::Active { name, .. } => {
                Some(format!("@{}", name.trim_end_matches(".dash")))
            }
            _ => None,
        })
        .collect();
    let mut action = AppAction::None;
    let mut outcome = HomeOutcome::None;
    let dark = ui.visuals().dark_mode;
    let wide = ui.available_width() >= 640.0;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            if let Some(url) = avatar_url.filter(|url| !avatars.is_failed(url)) {
                if let Some(task) = Avatar::new(Some(url), 56.0).show(ui, avatars).fetch {
                    pending_avatars.push(task);
                }
            } else {
                super::avatar::paint_identity_monogram(
                    ui,
                    56.0,
                    identity.identity_type.into(),
                    heading.chars().next(),
                    DashColors::DASH_BLUE,
                );
            }
            ui.add_space(Spacing::SM);
            let width = (ui.available_width() - if wide { 255.0 } else { 0.0 }).max(100.0);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 56.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(width);
                    ui.add(
                        egui::Label::new(RichText::new(&heading).size(20.0).strong()).truncate(),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if names.is_empty()
                            && let Some(request) = requests.iter().find(|r| r.phase.is_pending())
                        {
                            if ui.link(format!("@{}", request.label)).clicked() {
                                outcome = HomeOutcome::GoToUsernames;
                            }
                            ui.label(super::username_copy::phase_label(request.phase));
                        } else if names.is_empty() {
                            ui.label(
                                RichText::new("No username yet")
                                    .color(DashColors::text_secondary(dark)),
                            );
                        } else {
                            ui.label(names.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
                        }
                        if names.len() > 3
                            && ui
                                .button("…")
                                .on_hover_text("View all usernames.")
                                .clicked()
                        {
                            outcome = HomeOutcome::GoToUsernames;
                        }
                        let add = ui
                            .add(
                                ComponentStyles::secondary_button("+", dark)
                                    .min_size(egui::vec2(28.0, 28.0)),
                            )
                            .on_hover_text("Get another username.");
                        add.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                "Add username",
                            )
                        });
                        if add.clicked() {
                            action = register_action_for(app_context, identity.identity.id());
                        }
                    });
                },
            );
            if wide {
                summary_balance(ui, app_context, &identity, &mut action, &mut outcome);
            }
        });
        if !wide {
            ui.add_space(Spacing::SM);
            summary_balance(ui, app_context, &identity, &mut action, &mut outcome);
        }
    });
    (action, outcome)
}

fn summary_balance(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    action: &mut AppAction,
    outcome: &mut HomeOutcome,
) {
    let dark = ui.visuals().dark_mode;
    ui.vertical(|ui| {
        ui.label(
            RichText::new("Identity balance")
                .small()
                .color(DashColors::text_secondary(dark)),
        )
        .on_hover_text("This balance pays for usernames, profile updates, contacts, and payments.");
        ui.label(
            RichText::new(format_credits_as_dash(identity.identity.balance()))
                .size(20.0)
                .monospace(),
        );
        ui.horizontal(|ui| {
            let withdraw = ui.add_enabled(
                identity.can_attempt_withdrawal(app_context.user_role()),
                ComponentStyles::secondary_button("Move to wallet", dark),
            );
            if withdraw.disabled_tooltip("This identity has no key available for withdrawal in the current interface mode. Add an eligible key to continue.").clicked() {
                *action = AppAction::AddScreen(
                    ScreenType::IdentitySendScreen(identity.clone(), IdentitySendMode::MoveToWallet)
                        .create_screen(app_context),
                );
            }
            ui.menu_button("•••", |ui| {
                if ui.button("Advanced settings").clicked() {
                    *outcome = HomeOutcome::GoToAdvanced;
                    ui.close();
                }
                if ui.button("View usernames").clicked() {
                    *outcome = HomeOutcome::GoToUsernames;
                    ui.close();
                }
            });
        });
    });
}

/// Home content below the identity summary and tabs. Its data loads are not
/// part of rendering: see [`load_action`].
pub fn render(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    state: &mut HomeState,
    profiles: &mut ProfileCache,
    contacts: &mut ContactsState,
) -> (AppAction, HomeOutcome) {
    let Some(identity) = app_context.resolve_selected_identity() else {
        return (AppAction::None, HomeOutcome::None);
    };
    state.reconcile_owner(&identity);
    contacts.reconcile_owner(&identity);
    let mut action = AppAction::None;
    let mut outcome = HomeOutcome::None;
    // `None` until the profile is read.
    let has_profile = profiles.get_or_request(&identity).map(|profile| {
        profile
            .as_ref()
            .and_then(|p| p.display_name_opt())
            .is_some()
    });
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if ui.available_width() >= 760.0 {
                let content_width = ui.available_width() - Spacing::LG;
                // A05 uses a 620:360 split, with contacts beside the action row.
                let left_width = content_width * 620.0 / 980.0;
                let right_width = content_width - left_width;
                ui.spacing_mut().item_spacing.x = Spacing::LG;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(left_width);
                            (action, outcome) = render_main_column(
                                ui,
                                app_context,
                                &identity,
                                state,
                                has_profile,
                                contacts,
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(right_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(right_width);
                            action |= super::contacts::render_preview(
                                ui,
                                app_context,
                                &identity,
                                contacts,
                                &mut outcome,
                            );
                        },
                    );
                });
            } else {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    (action, outcome) = render_main_column(
                        ui,
                        app_context,
                        &identity,
                        state,
                        has_profile,
                        contacts,
                    );
                    action |= super::contacts::render_preview(
                        ui,
                        app_context,
                        &identity,
                        contacts,
                        &mut outcome,
                    );
                });
            }
        });
    if state.receive_open {
        action |= render_receive(ui, app_context, &identity, state);
    }
    (action, outcome)
}

/// Home's next due load for the selected identity: its contact lists, then its
/// payment history.
///
/// Returning a load claims its one dispatch slot, so the caller must ask only
/// once nothing else on the screen produced an action this frame, and must
/// return what it gets: a load that is replaced never goes out again.
pub fn load_action(
    app_context: &Arc<AppContext>,
    state: &mut HomeState,
    contacts: &mut ContactsState,
) -> AppAction {
    if !contacts.load_is_due() && !state.payments_load_is_due() {
        return AppAction::None;
    }
    let Some(identity) = app_context.resolve_selected_identity() else {
        return AppAction::None;
    };
    state.reconcile_owner(&identity);
    contacts.reconcile_owner(&identity);
    let action = super::contacts::load_action(&identity, contacts);
    if matches!(action, AppAction::None) {
        state.load_payments(&identity)
    } else {
        action
    }
}

fn render_main_column(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut HomeState,
    has_profile: Option<bool>,
    contacts: &ContactsState,
) -> (AppAction, HomeOutcome) {
    let mut action = AppAction::None;
    let dark = ui.visuals().dark_mode;
    ui.horizontal_wrapped(|ui| {
        if ComponentStyles::add_primary_button(ui, "Send").clicked() {
            action = AppAction::AddScreen(
                ScreenType::IdentitySendScreen(identity.clone(), IdentitySendMode::Send)
                    .create_screen(app_context),
            );
        }
        if ComponentStyles::add_secondary_button(ui, "Receive", dark).clicked() {
            state.receive_open = true;
        }
        if ComponentStyles::add_secondary_button(ui, "Top up", dark).clicked() {
            action = AppAction::AddScreen(
                ScreenType::IdentitySendScreen(identity.clone(), IdentitySendMode::TopUp)
                    .create_screen(app_context),
            );
        }
    });
    ui.add_space(Spacing::MD);
    let outcome = render_setup(
        ui,
        app_context,
        identity,
        state,
        has_profile,
        contacts,
        &mut action,
    );
    let notice_action = super::username_notices::render(ui, app_context, identity);
    if matches!(action, AppAction::None) {
        action = notice_action;
    }
    action |= render_payments(ui, app_context, state);
    (action, outcome)
}

fn render_setup(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &HomeState,
    has_profile: Option<bool>,
    contacts: &ContactsState,
    action: &mut AppAction,
) -> HomeOutcome {
    if state.dismissed_checklist {
        return HomeOutcome::None;
    }
    // An input still loading is not an unfinished step: show nothing until
    // both are read, so the checklist never flashes up and vanishes.
    let Some(has_profile) = has_profile else {
        return HomeOutcome::None;
    };
    if !contacts.contacts_known() {
        return HomeOutcome::None;
    }
    let mut checklist = OnboardingChecklist::new();
    if let Some(name) = app_context.main_username(identity) {
        checklist = checklist
            .with_handle(&name)
            .mark_complete(ChecklistStep::PickUsername);
    } else if let Some(request) = app_context.pending_dpns_username_for_identity(identity) {
        checklist = checklist.with_pending_username(request.label);
    }
    if has_profile {
        checklist = checklist.mark_complete(ChecklistStep::SetDisplayName);
    }
    if contacts.contacts_len() > 0 {
        checklist = checklist.mark_complete(ChecklistStep::AddFirstContact);
    }
    let mut outcome = HomeOutcome::None;
    if !checklist.all_complete() {
        match checklist.show(ui).action() {
            Some(ChecklistAction::Dismissed) => outcome = HomeOutcome::DismissChecklist,
            Some(ChecklistAction::Activated(ChecklistStep::PickUsername)) => {
                *action = register_action_for(app_context, identity.identity.id())
            }
            Some(ChecklistAction::Activated(ChecklistStep::SetDisplayName)) => {
                outcome = HomeOutcome::GoToSettings
            }
            Some(ChecklistAction::Activated(ChecklistStep::AddFirstContact)) => {
                outcome = HomeOutcome::GoToContacts
            }
            None => {}
        }
        ui.add_space(Spacing::MD);
    }
    outcome
}

fn render_payments(ui: &mut Ui, app_context: &Arc<AppContext>, state: &HomeState) -> AppAction {
    if state.payments.is_empty() {
        return AppAction::None;
    }
    let mut action = AppAction::None;
    card(ui, |ui| {
        ui.strong("Recent payments");
        for (_, contact, amount, incoming, _) in state.payments.iter().take(5) {
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::model::identity_name::clean_display_text(contact));
                let amount = format_duffs_as_dash(*amount);
                ui.label(if *incoming {
                    format!("Received {amount}")
                } else {
                    format!("Sent {amount}")
                });
            });
        }
        if ui.button("See all payments").clicked() {
            action = AppAction::AddScreen(ScreenType::DashPayPayments.create_screen(app_context));
        }
    });
    ui.add_space(Spacing::MD);
    action
}

fn render_receive(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    identity: &QualifiedIdentity,
    state: &mut HomeState,
) -> AppAction {
    let mut action = AppAction::None;
    let response = egui::Modal::new(egui::Id::new("identity_receive")).show(ui.ctx(), |ui| {
        ui.set_max_width(400.0);
        ui.heading("Receive Dash");
        let name = app_context.main_username(identity);
        let share = name
            .as_ref()
            .map(|name| format!("{}.dash", name.trim_end_matches(".dash")))
            .unwrap_or_else(|| identity.identity.id().to_string(Encoding::Base58));
        if let Some(name) = name {
            ui.strong(format!("@{name}"));
            ui.label("Share your username with someone sending to your identity.");
            if ui.button("Copy username").clicked() {
                ui.ctx().copy_text(share.clone());
            }
        } else {
            ui.label("Get a username so people can find your identity by name.");
            if ui.button("Get a username").clicked() {
                action = register_action_for(app_context, identity.identity.id());
                state.receive_open = false;
            }
            ui.label("You can also share your identity ID.");
            if ui.button("Copy identity ID").clicked() {
                ui.ctx().copy_text(share.clone());
            }
        }
        if state.receive_qr.is_none()
            && let Ok(image) = super::funding_common::generate_qr_code_image(&share)
        {
            state.receive_qr = Some(ui.ctx().load_texture(
                "identity_receive_qr",
                image,
                egui::TextureOptions::NEAREST,
            ));
        }
        if let Some(texture) = &state.receive_qr {
            ui.image((texture.id(), egui::vec2(200.0, 200.0)));
        }
        if ui.button("Done").clicked() {
            state.receive_open = false;
        }
    });
    if response.should_close() {
        state.receive_open = false;
    }
    if !state.receive_open {
        state.receive_qr = None;
    }
    action
}

pub(super) fn card<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::group(ui.style())
        .fill(DashColors::surface(ui.visuals().dark_mode))
        .corner_radius(Shape::RADIUS_MD)
        .inner_margin(Margin::same(Spacing::MD as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui)
        })
        .inner
}

pub fn apply_outcome(state: &mut HomeState, outcome: HomeOutcome) -> Option<IdentityHubTab> {
    match outcome {
        HomeOutcome::None => None,
        HomeOutcome::GoToContacts => Some(IdentityHubTab::Contacts),
        HomeOutcome::GoToSettings | HomeOutcome::GoToUsernames | HomeOutcome::GoToAdvanced => {
            Some(IdentityHubTab::Settings)
        }
        HomeOutcome::DismissChecklist => {
            state.dismissed_checklist = true;
            None
        }
    }
}
