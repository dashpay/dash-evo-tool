//! The sequential Token Creator workspace and screen-specific controls.

mod basics;
mod dialogs;
mod distribution;
mod documents;
mod groups;
mod review;
mod supply;
mod wallet;

use super::TokensScreen;
use crate::app::AppAction;
use crate::model::token_creator::amount::{TokenAmountInput, format_amount};
use crate::model::token_creator::rules::{ActorDraft, RuleKind};
use crate::model::token_creator::{Policy, Recipient, Section};
use crate::ui::state::token_creator::CreatorWorkspace;
use crate::ui::theme::ComponentStyles;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use egui::{Context, RichText, Ui};

const TUTORIAL_URL: &str =
    "https://drive.google.com/drive/folders/1ECWS2d_-OeHXTW-Ur-qFlf_mZzYUIqQ1?usp=sharing";

impl TokensScreen {
    pub(super) fn render_token_workspace(&mut self, context: &Context, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        self.creator_workspace.draft.reconcile_policy();
        if self
            .creator_workspace
            .reviewed_draft
            .as_ref()
            .is_some_and(|draft| *draft != self.creator_workspace.draft)
        {
            self.creator_workspace.prepared = None;
            self.creator_workspace.acknowledged = false;
        }
        ui.horizontal(|ui| {
            ui.heading("Create a token");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.hyperlink_to("Video tutorial ↗", TUTORIAL_URL);
            });
        });
        ui.label("Start with its purpose. Decide what can change later.");
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Network: {}", self.app_context.network));
            if let Some(identity) = &self.selected_identity {
                ui.label(format!("Owner: {}", identity.display_string()));
            } else {
                ui.label("Drafting is available. Choose an identity in Review when you are ready.");
            }
            help(ui, &mut self.creator_workspace.help, "readiness");
        });
        ui.add_space(12.0);
        let navigation = (
            self.creator_workspace.section,
            ui.available_width().to_bits(),
        );
        let reveal_selected = self.creator_workspace.last_navigation != Some(navigation);
        self.creator_workspace.last_navigation = Some(navigation);
        egui::ScrollArea::horizontal()
            .id_salt("creator_sections")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, section) in Section::ALL.into_iter().enumerate() {
                        let selected = self.creator_workspace.section == section;
                        let response = ui.add(
                            egui::Button::selectable(
                                selected,
                                format!("{}  {}", index + 1, section.label()),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                        if response.clicked() {
                            self.creator_workspace.section = section;
                        }
                        if selected && reveal_selected {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                    }
                });
            });
        ui.add_space(16.0);
        if let Some(error) = &self.creator_workspace.error {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{}: {error}", error.field),
                );
                if ui
                    .link(format!("Go to {}", error.section.label()))
                    .clicked()
                {
                    self.creator_workspace.section = error.section;
                }
            });
        }
        if let Some(notice) = &self.creator_workspace.notice {
            ui.horizontal_wrapped(|ui| {
                ui.label(notice);
                if let Some(section) = self.creator_workspace.error_section
                    && ui.link(format!("Go to {}", section.label())).clicked()
                {
                    self.creator_workspace.section = section;
                }
            });
        }
        let wide = ui.available_width() >= 760.0;
        if !wide {
            egui::CollapsingHeader::new("Your token · draft summary").show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("compact_summary")
                    .max_height(160.0)
                    .show(ui, |ui| summary(ui, &mut self.creator_workspace));
            });
        }
        let width = ui.available_width() - if wide { 248.0 } else { 0.0 };
        let body_height = (ui.available_height() - 48.0).max(100.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(width);
                    egui::ScrollArea::vertical()
                        .id_salt(("creator_editor", self.creator_workspace.section.label()))
                        .auto_shrink([false, false])
                        .max_height(body_height)
                        .show(ui, |ui| match self.creator_workspace.section {
                            Section::Basics => basics::show(ui, &mut self.creator_workspace),
                            Section::Supply => supply::show(
                                ui,
                                &mut self.creator_workspace,
                                self.app_context.platform_version(),
                            ),
                            Section::Distribution => distribution::show(
                                ui,
                                &mut self.creator_workspace,
                                self.app_context.network,
                                self.app_context.platform_version(),
                            ),
                            Section::Groups => groups::show(
                                ui,
                                &mut self.creator_workspace,
                                self.app_context.platform_version(),
                            ),
                            Section::History => documents::history(ui, &mut self.creator_workspace),
                            Section::Documents => documents::show(ui, &mut self.creator_workspace),
                            Section::Review => action |= self.render_workspace_review(ui),
                        });
                },
            );
            if wide {
                ui.add_space(8.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(232.0, body_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(232.0);
                        egui::ScrollArea::vertical()
                            .id_salt("creator_summary")
                            .auto_shrink([false, false])
                            .max_height(body_height)
                            .show(ui, |ui| summary(ui, &mut self.creator_workspace));
                    },
                );
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            let position = Section::ALL
                .iter()
                .position(|section| *section == self.creator_workspace.section)
                .unwrap_or(0);
            if ui
                .add_enabled(
                    self.creator_workspace.section != Section::Basics,
                    egui::Button::new("← Back").min_size(egui::vec2(80.0, 32.0)),
                )
                .clicked()
            {
                self.creator_workspace.navigate(false);
            }
            ui.small(format!(
                "Step {step} of 7. Your draft is kept as you continue.",
                step = position + 1
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(next) = Section::ALL.get(position + 1)
                    && ComponentStyles::add_primary_button(
                        ui,
                        format!("Continue: {} →", next.label()),
                    )
                    .clicked()
                {
                    self.creator_workspace.navigate(true);
                }
            });
        });
        dialogs::show(context, &mut self.creator_workspace);
        action
    }
}

fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().panel_fill)
        .inner_margin(16)
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
    ui.add_space(12.0);
}

const HELP_OPENER: &str = "creator_help_opener";

fn help(ui: &mut Ui, topic: &mut Option<String>, key: &str) {
    let name = format!("Help: {}", dialogs::topic_title(key));
    let response = ui
        .add(egui::Button::new("?").min_size(egui::vec2(32.0, 32.0)))
        .on_hover_text(&name);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &name)
    });
    let restore = ui.ctx().data(|data| {
        data.get_temp::<bool>(egui::Id::new("creator_restore_help_focus")) == Some(true)
            && data.get_temp::<egui::Id>(egui::Id::new(HELP_OPENER)) == Some(response.id)
    });
    if restore {
        if ui.memory(|memory| memory.allows_interaction(response.layer_id)) {
            response.request_focus();
            ui.ctx().data_mut(|data| {
                data.remove::<bool>(egui::Id::new("creator_restore_help_focus"));
                data.remove::<egui::Id>(egui::Id::new(HELP_OPENER));
            });
        } else {
            // Modal focus restrictions clear at frame end; wait until the opener's layer is active.
            ui.ctx().request_repaint();
        }
    }
    if response.clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new(HELP_OPENER), response.id));
        *topic = Some(key.into());
    }
}

fn heading(ui: &mut Ui, text: &str, key: &str, topic: &mut Option<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.heading(text);
        help(ui, topic, key);
    });
    ui.add_space(6.0);
}

fn label(ui: &mut Ui, text: &str, key: &str, topic: &mut Option<String>) -> egui::Response {
    ui.horizontal_wrapped(|ui| {
        let response = ui.strong(text);
        help(ui, topic, key);
        response
    })
    .inner
}

fn text(ui: &mut Ui, caption: &str, key: &str, value: &mut String, topic: &mut Option<String>) {
    let caption = label(ui, caption, key, topic);
    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
        .labelled_by(caption.id);
    ui.add_space(8.0);
}

fn checkbox(ui: &mut Ui, caption: &str, key: &str, value: &mut bool, topic: &mut Option<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(value, caption);
        help(ui, topic, key);
    });
}

fn amount(
    ui: &mut Ui,
    caption: &str,
    key: &str,
    value: &mut TokenAmountInput,
    decimals: u8,
    topic: &mut Option<String>,
) {
    let caption_response = label(ui, caption, key, topic);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut value.text).desired_width(180.0))
            .labelled_by(caption_response.id);
        let mut raw = value.base_units;
        egui::ComboBox::from_id_salt((caption, "units"))
            .selected_text(if raw { "base units" } else { "tokens" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut raw, false, "tokens");
                ui.selectable_value(&mut raw, true, "base units");
            })
            .response
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::ComboBox,
                    ui.is_enabled(),
                    format!("{caption} units"),
                )
            });
        if let Err(error) = value.change_units(raw, decimals) {
            ui.colored_label(ui.visuals().error_fg_color, error.to_string());
        }
    });
    if !value.text.trim().is_empty() {
        match value.parse(decimals) {
            Ok(raw) => {
                ui.small(format!(
                    "{} tokens = {raw} base units",
                    format_amount(raw, decimals)
                ));
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            }
        }
    }
    ui.add_space(8.0);
}

fn policy_label(policy: Policy) -> &'static str {
    match policy {
        Policy::Unselected => "Supply policy not chosen",
        Policy::Fixed => "Fixed supply",
        Policy::LimitedOwner => "Owner-managed, with a hard cap",
        Policy::Custom => "Custom rules",
        Policy::Preset(features) => preset_label(features),
    }
}

fn preset_label(
    features: dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures,
) -> &'static str {
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::*;
    match features {
        MostRestrictive => "Most restrictive",
        WithOnlyEmergencyAction => "Emergency action",
        WithMintingAndBurningActions => "Minting and burning",
        WithExtremeActions => "Extreme actions",
        WithAllAdvancedActions => "All advanced actions",
    }
}

fn recipient_label(recipient: &Recipient) -> String {
    match recipient {
        Recipient::Owner => "Contract owner".into(),
        Recipient::None => "No fixed destination (initial supply goes to the owner)".into(),
        Recipient::Identity(id) => format!("Identity {id}"),
        Recipient::EvoNodes => "EvoNodes by participation".into(),
    }
}

fn summary(ui: &mut Ui, state: &mut CreatorWorkspace) {
    state.draft.reconcile_policy();
    card(ui, |ui| {
        heading(ui, "Your token", "review", &mut state.help);
        ui.label(
            RichText::new(state.draft.names.first().map_or("Untitled token", |name| {
                if name.singular.is_empty() {
                    "Untitled token"
                } else {
                    &name.singular
                }
            }))
            .strong(),
        );
        ui.label(policy_label(state.draft.policy));
        ui.separator();
        for (label, amount) in [
            ("Initial supply", &state.draft.initial),
            ("Maximum supply", &state.draft.maximum),
        ] {
            if label == "Maximum supply" && state.draft.no_maximum {
                ui.label("Maximum supply: no configured limit");
            } else {
                ui.label(format!(
                    "{label}: {}",
                    amount
                        .parse(state.draft.decimals)
                        .map(|raw| format!("{} tokens", format_amount(raw, state.draft.decimals)))
                        .unwrap_or_else(|_| "not valid yet".into())
                ));
            }
        }
        ui.small(supply::cap_consequence(&state.draft));
        ui.small(format!("{} decimal places", state.draft.decimals));
        ui.label(format!(
            "Initial recipient: {}",
            recipient_label(&state.draft.mint_recipient)
        ));
        ui.separator();
        ui.label(if state.draft.transferable {
            "Transfers: allowed · permanent choice"
        } else {
            "Transfers: not allowed · permanent choice"
        });
        if state.draft.start_paused {
            ui.small("Transfers start paused.");
        }
        ui.separator();
        ui.strong("Distribution");
        ui.label(format!(
            "Recurring: {}",
            on_off(state.draft.recurring.enabled)
        ));
        ui.small(distribution::recurring_authority(&state.draft));
        ui.label(format!(
            "Scheduled: {status} · {count} saved allocations",
            status = on_off(state.draft.scheduled_enabled),
            count = state.draft.scheduled.len()
        ));
        ui.label(format!(
            "Once per identity: {}",
            on_off(state.draft.once_enabled)
        ));
        ui.label(format!("Groups: {}", state.draft.groups.len()));
        ui.label(format!(
            "History: {count} of 6",
            count = state
                .draft
                .history
                .iter()
                .filter(|enabled| **enabled)
                .count()
        ));
        ui.separator();
        ui.strong("Powers and consequences");
        for kind in RuleKind::ALL {
            let rule = state.draft.permissions.get(kind);
            if rule.performer != ActorDraft::NoOne
                || rule.administrator != ActorDraft::NoOne
                || !rule.permanently_disabled()
            {
                ui.label(kind.label());
                ui.small(format!("Performer: {}", rule.performer.label()));
                ui.small(format!("Administrator: {}", rule.administrator.label()));
                if rule.allow_disable_action
                    || rule.allow_remove_administrator
                    || rule.allow_replace_administrator
                {
                    ui.small("Later changes are allowed. Review the rule's three allowances.");
                }
            }
        }
        if state
            .draft
            .permissions
            .rules
            .iter()
            .all(|rule| rule.permanently_disabled())
        {
            ui.label("All 13 action rules are permanently disabled.");
        }
        ui.separator();
        ui.small("Creation fees come from the issuer's Platform balance. Review all permanent choices before signing.");
        ui.small("Draft only. Nothing is created or charged before the final confirmation.");
    });
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "On" } else { "Off" }
}
